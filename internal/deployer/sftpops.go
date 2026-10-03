package deployer

import (
	"context"
	"io"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/pkg/sftp"
	"golang.org/x/crypto/ssh"
	"golang.org/x/crypto/ssh/knownhosts"
)

// knownHostsCallback 加载 known_hosts 文件构造严格校验回调。
func knownHostsCallback(path string) (ssh.HostKeyCallback, error) {
	if _, err := os.Stat(path); err != nil {
		return nil, deployErrf("开启严格主机校验需要 known_hosts 文件: %s", path)
	}
	cb, err := knownhosts.New(path)
	if err != nil {
		return nil, deployErrf("解析 known_hosts 失败 %s: %v", path, err)
	}
	return cb, nil
}

// ============================================================
// 远端目录与备份（纯逻辑，便于单测）
// ============================================================

// remoteSegments 把远端路径拆成逐级目录片段。
// 绝对路径保持绝对，相对路径保持相对（修复了原版一律补前导 / 的问题）。
func remoteSegments(dir string) (abs bool, segs []string) {
	d := strings.Trim(dir, " ")
	if d == "" || d == "/" {
		return false, nil
	}
	abs = strings.HasPrefix(d, "/")
	for _, s := range strings.Split(strings.Trim(d, "/"), "/") {
		if s == "" || s == "." || s == ".." {
			continue
		}
		segs = append(segs, s)
	}
	return abs, segs
}

// ensureRemoteDir 逐级创建远端目录（已存在则跳过）。
func ensureRemoteDir(c *sftp.Client, remoteDir string) error {
	abs, segs := remoteSegments(remoteDir)
	if len(segs) == 0 {
		return nil
	}
	cur := ""
	if abs {
		cur = "/"
	}
	for _, seg := range segs {
		if cur == "" {
			cur = seg
		} else if strings.HasSuffix(cur, "/") {
			cur += seg
		} else {
			cur += "/" + seg
		}
		if _, err := c.Stat(cur); err == nil {
			continue
		}
		if err := c.Mkdir(cur); err != nil {
			// 并发创建 / 权限不足后复查，已存在则忽略
			if _, statErr := c.Stat(cur); statErr != nil {
				return deployErrf("创建远端目录失败 %s: %v", cur, err)
			}
		}
	}
	return nil
}

// filterBackups 筛出指定前缀的备份文件名，按名称倒序（等价于时间倒序）。
func filterBackups(entries []string, prefix string) []string {
	out := make([]string, 0, len(entries))
	for _, n := range entries {
		if strings.HasPrefix(n, prefix) {
			out = append(out, n)
		}
	}
	sort.Sort(sort.Reverse(sort.StringSlice(out)))
	return out
}

// backupsToRemove 返回超出保留数量、需要删除的备份名。
// keep <= 0 表示不清理。
func backupsToRemove(backups []string, keep int) []string {
	if keep <= 0 || len(backups) <= keep {
		return nil
	}
	return backups[keep:]
}

// cleanupOldBackups 删除超出 keep 的历史备份。
func cleanupOldBackups(c *sftp.Client, remoteDir, remoteName string, keep int, log LogFn) error {
	if keep <= 0 {
		return nil
	}
	entries, err := c.ReadDir(remoteDir)
	if err != nil {
		return nil // 目录不可读时静默跳过（与 Python 一致）
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		names = append(names, e.Name())
	}
	backups := filterBackups(names, remoteName+".bak.")
	for _, name := range backupsToRemove(backups, keep) {
		if err := c.Remove(remoteJoin(remoteDir, name)); err != nil {
			if log != nil {
				log("清理旧备份失败 " + name + ": " + err.Error())
			}
			continue
		}
		if log != nil {
			log("清理旧备份: " + name)
		}
	}
	return nil
}

// ============================================================
// 上传
// ============================================================

// progressWriter 统计已写字节并回调。
type progressWriter struct {
	dst    io.Writer
	total  int64
	copied int64
	cb     ProgressFn
	ctx    context.Context
}

func (w *progressWriter) Write(p []byte) (int, error) {
	if w.ctx != nil {
		select {
		case <-w.ctx.Done():
			return 0, w.ctx.Err()
		default:
		}
	}
	n, err := w.dst.Write(p)
	w.copied += int64(n)
	if w.cb != nil && w.total > 0 {
		w.cb(w.copied, w.total)
	}
	return n, err
}

// uploadJar 执行"上传 → 备份旧包 → 原子替换 → 清理历史备份"全流程。
func uploadJar(
	ctx context.Context,
	c *sftp.Client,
	localJar, remoteDir, remoteName string,
	log LogFn,
	progress ProgressFn,
	backupKeep int,
	now time.Time,
) error {
	fi, err := os.Stat(localJar)
	if err != nil || fi.IsDir() {
		return deployErrf("本地 jar 不存在: %s", localJar)
	}

	if err := ensureRemoteDir(c, remoteDir); err != nil {
		return err
	}

	remoteFinal := remoteJoin(remoteDir, remoteName)
	remoteTmp := remoteFinal + ".tmp"
	size := fi.Size()

	if log != nil {
		log("上传 " + localJar + " (" + strconv.FormatInt(size, 10) + " bytes) -> " + remoteFinal)
	}

	src, err := os.Open(localJar)
	if err != nil {
		return deployErrf("读取本地 jar 失败: %v", err)
	}
	defer src.Close()

	dst, err := c.Create(remoteTmp)
	if err != nil {
		return deployErrf("创建远端临时文件失败 %s: %v", remoteTmp, err)
	}
	pw := &progressWriter{dst: dst, total: size, cb: progress, ctx: ctx}
	if _, err := io.Copy(pw, src); err != nil {
		_ = dst.Close()
		_ = c.Remove(remoteTmp)
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return deployErrf("上传中断: %v", err)
	}
	if err := dst.Close(); err != nil {
		_ = c.Remove(remoteTmp)
		return deployErrf("写入远端文件失败: %v", err)
	}

	// 旧文件 → 备份
	if _, err := c.Stat(remoteFinal); err == nil {
		backup := backupName(remoteName, timestampMs(now))
		backupPath := remoteJoin(remoteDir, backup)
		if err := c.Rename(remoteFinal, backupPath); err == nil && log != nil {
			log("已备份旧文件: " + backupPath)
		}
	}

	// 临时文件 → 正式文件（原子替换）
	if err := c.Rename(remoteTmp, remoteFinal); err != nil {
		return deployErrf("替换远端文件失败: %v", err)
	}
	if log != nil {
		log("上传完成: " + remoteFinal)
	}

	return cleanupOldBackups(c, remoteDir, remoteName, backupKeep, log)
}
