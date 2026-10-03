package deployer

import (
	"context"
	"fmt"
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

// remoteSegments 把**已规范化**的远端路径拆成逐级目录片段。
// 绝对路径保持绝对，相对路径保持相对（不一律补前导 /）。
// ".." 原样保留：相对路径允许以 .. 开头，与 Python _ensure_remote_dir 一致。
func remoteSegments(dir string) (abs bool, segs []string) {
	abs = strings.HasPrefix(dir, "/")
	trimmed := strings.Trim(dir, "/")
	if trimmed == "" {
		return abs, nil
	}
	for _, s := range strings.Split(trimmed, "/") {
		if s == "" || s == "." {
			continue
		}
		segs = append(segs, s)
	}
	return abs, segs
}

// ensureRemoteDir 规范化、展开 `~` 并逐级创建远端目录，
// 返回**实际落点路径**（后续临时文件 / 备份 / 正式文件都必须用它，
// 否则会出现"目录建在 A 处、文件写进 B 处"的错位）。
// 可安全并发调用。
func ensureRemoteDir(c *sftp.Client, remoteDir string) (string, error) {
	normalized, err := NormalizeRemoteDir(remoteDir)
	if err != nil {
		return "", err
	}
	path, err := ExpandRemoteHome(c, normalized)
	if err != nil {
		return "", err
	}
	if path == "/" {
		return path, nil
	}

	abs, segs := remoteSegments(path)
	if len(segs) == 0 {
		return path, nil
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
			// TOCTOU：stat 与 mkdir 之间该层可能已被并发部署的连接创建，
			// 复查一次存在即可；仍不存在才把失败明确抛出去。
			if _, statErr := c.Stat(cur); statErr != nil {
				return "", deployErrf("创建远端目录失败 %s: %v", cur, err)
			}
		}
	}
	return path, nil
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

// cleanupOldFiles 按 `<remote_name>.<marker>.<时间戳>` 前缀清理历史文件，
// 每类最多保留 keep 份。
//
// marker 取值：
//   - bak    部署前的旧版本备份（回滚的来源），由 uploadJar 生成
//   - failed 重启失败后被隔离的新版本，由 restoreBackup 生成，留证用
//
// keep <= 0 表示不清理。
func cleanupOldFiles(c *sftp.Client, remoteDir, remoteName, marker string, keep int, log LogFn) error {
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
	prefix := remoteName + "." + marker + "."
	backups := filterBackups(names, prefix)

	label := "备份"
	if marker != "bak" {
		label = "失败残留"
	}
	for _, name := range backupsToRemove(backups, keep) {
		if err := c.Remove(remoteJoin(remoteDir, name)); err != nil {
			if log != nil {
				log(fmt.Sprintf("清理旧%s失败 %s: %v", label, name, err))
			}
			continue
		}
		if log != nil {
			log("清理旧" + label + ": " + name)
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

// uploadOutcome 一次上传的落点信息，供部署失败时精确回滚。
type uploadOutcome struct {
	// RemoteDir 已规范化、已展开 ~ 的实际远端目录
	RemoteDir string
	// RemoteName 实际使用的远端文件名
	RemoteName string
	// BackupPath 被替换下来的部署前版本；首次部署为空
	BackupPath string
}

// uploadJar 执行"上传 → 备份旧包 → 原子替换 → 清理历史备份"全流程。
//
// 返回本次上传的落点信息（含备份路径），供重启失败后的自动回滚使用。
func uploadJar(
	ctx context.Context,
	c *sftp.Client,
	localJar, remoteDir, remoteName string,
	log LogFn,
	progress ProgressFn,
	backupKeep int,
	now time.Time,
) (*uploadOutcome, error) {
	fi, err := os.Stat(localJar)
	if err != nil || fi.IsDir() {
		return nil, deployErrf("本地 jar 不存在: %s", localJar)
	}
	if remoteName == "" {
		return nil, deployErrf("远端文件名为空，无法确定落点")
	}

	// 规范化后的路径必须贯穿备份 / 临时文件 / 目录创建，
	// 否则三者可能指向不同位置。
	remoteDir, err = ensureRemoteDir(c, remoteDir)
	if err != nil {
		return nil, err
	}

	remoteFinal := remoteJoin(remoteDir, remoteName)
	remoteTmp := remoteFinal + ".tmp"
	size := fi.Size()

	if log != nil {
		log("上传 " + localJar + " (" + strconv.FormatInt(size, 10) + " bytes) -> " + remoteFinal)
	}

	src, err := os.Open(localJar)
	if err != nil {
		return nil, deployErrf("读取本地 jar 失败: %v", err)
	}
	defer src.Close()

	dst, err := c.Create(remoteTmp)
	if err != nil {
		return nil, deployErrf("创建远端临时文件失败 %s: %v", remoteTmp, err)
	}
	pw := &progressWriter{dst: dst, total: size, cb: progress, ctx: ctx}
	if _, err := io.Copy(pw, src); err != nil {
		_ = dst.Close()
		_ = c.Remove(remoteTmp)
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, deployErrf("上传中断: %v", err)
	}
	if err := dst.Close(); err != nil {
		_ = c.Remove(remoteTmp)
		return nil, deployErrf("写入远端文件失败: %v", err)
	}

	// 旧文件 → 备份
	backupPath := ""
	if _, err := c.Stat(remoteFinal); err == nil {
		backup := backupName(remoteName, timestampMs(now))
		bp := remoteJoin(remoteDir, backup)
		if err := c.Rename(remoteFinal, bp); err == nil {
			backupPath = bp
			if log != nil {
				log("已备份旧文件: " + bp)
			}
		}
	}

	// 临时文件 → 正式文件（原子替换）
	if err := c.Rename(remoteTmp, remoteFinal); err != nil {
		// 旧文件已被改名为备份、新文件却未能就位时，把备份恢复回原名，
		// 避免线上 jar 缺失导致服务无法启动。
		if backupPath != "" {
			if rerr := c.Rename(backupPath, remoteFinal); rerr == nil {
				if log != nil {
					log("[回滚] 新文件替换失败，已恢复原文件: " + remoteFinal)
				}
			} else if log != nil {
				log(fmt.Sprintf(
					"[严重] 新文件替换失败且恢复原文件失败: %v。线上 %s 当前缺失，请手动处理备份: %s",
					rerr, remoteFinal, backupPath))
			}
		}
		return nil, deployErrf("替换远端文件失败: %v", err)
	}
	if log != nil {
		log("上传完成: " + remoteFinal)
	}

	_ = cleanupOldFiles(c, remoteDir, remoteName, "bak", backupKeep, log)
	// 失败残留同样按 backupKeep 封顶，避免多次回滚后在远端越积越多
	_ = cleanupOldFiles(c, remoteDir, remoteName, "failed", backupKeep, log)

	return &uploadOutcome{
		RemoteDir:  remoteDir,
		RemoteName: remoteName,
		BackupPath: backupPath,
	}, nil
}

// restoreBackup 把部署前的版本放回原位，返回被隔离留证的新版本路径。
//
// 两步 rename，与 uploadJar 里"替换失败恢复原文件"是同一套路：
//  1. 新版本挪到 `<name>.failed.<时间戳>` —— 让出目标名，同时保留现场
//  2. 部署前版本放回目标名
//
// 第 2 步失败时会把第 1 步挪走的新版本放回去，确保远端始终存在
// `name` 这个文件，不留下"jar 缺失"的中间态。
func restoreBackup(
	c *sftp.Client,
	remoteDir, remoteName, backupPath string,
	now time.Time,
	log LogFn,
) (string, error) {
	remoteFinal := remoteJoin(remoteDir, remoteName)
	failedPath := remoteJoin(remoteDir, remoteName+".failed."+timestampMs(now))

	if err := c.Rename(remoteFinal, failedPath); err != nil {
		return "", deployErrf("[回滚] 隔离新版本失败，部署前版本未恢复: %v", err)
	}
	if log != nil {
		log("[回滚] 已隔离本次上传的新版本: " + failedPath)
	}

	if err := c.Rename(backupPath, remoteFinal); err != nil {
		if rerr := c.Rename(failedPath, remoteFinal); rerr != nil {
			return "", deployErrf(
				"[回滚] 恢复部署前版本失败: %v；把新版本放回原位也失败: %v；"+
					"远端 %s 当前缺失，部署前版本仍在 %s，请立即手动处理",
				err, rerr, remoteFinal, backupPath)
		}
		return "", deployErrf(
			"[回滚] 恢复部署前版本失败: %v；已把新版本放回原位 %s；"+
				"部署前版本仍在 %s，请立即手动处理",
			err, remoteFinal, backupPath)
	}
	if log != nil {
		log("[回滚] 已恢复部署前版本: " + remoteFinal)
	}
	return failedPath, nil
}
