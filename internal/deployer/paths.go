package deployer

import (
	"regexp"
	"strings"

	"github.com/pkg/sftp"
)

// ============================================================
// 远端路径加固（移植自 Python legacy-python/src/core/deployer.py）
//
// 这一组函数解决的是同一类问题：配置里的远端路径最终会被拼进
// 远端 shell 命令，或者直接交给 SFTP 建目录，任何"原样拼接"
// 都会同时带来目录穿越与命令注入两条攻击面。
//
// 路径 → shell 的转义不在此处：基准把它放在 config.QuoteRemotePath
// （对应 Python config.py），重启命令与 tail/cp 共用同一套。
// ============================================================

// globSafePath 放行"必须交给远端 shell 做通配符展开"的日志路径。
//
// 这类路径无法整体加引号（加了引号 * 就不再是通配符），因此只能反向
// 白名单：只放行无需转义的字符，空格、引号、; & | $ ` 等一律拒绝。
// 换行同样被字符集排除在外 —— 它在 shell 里是命令分隔符。
var globSafePath = regexp.MustCompile(`^[A-Za-z0-9_./*?\[\]~-]+$`)

// NormalizeRemoteDir 规范化远端目录路径。
//
//   - 统一按 '/' 切分（反斜杠也当分隔符，避免 Windows 风格路径原样落到远端）
//   - 折叠重复斜杠、丢弃空段与 '.'，并**向上消化** '..'
//   - 保留绝对 / 相对语义：原本相对的路径仍然相对 SFTP 家目录，
//     与 shell 里 `cd remote_dir` 的行为一致
//   - 空路径直接报错，绝不落到 `mkdir /` 这种危险操作上
//
// 与 Python _normalize_remote_dir 逐行对应。
func NormalizeRemoteDir(remoteDir string) (string, error) {
	raw := strings.ReplaceAll(strings.TrimSpace(remoteDir), "\\", "/")
	if raw == "" {
		return "", deployErrf("remote_dir 为空，无法确定远端目录")
	}

	absolute := strings.HasPrefix(raw, "/")
	var segs []string
	for _, seg := range strings.Split(raw, "/") {
		if seg == "" || seg == "." {
			continue
		}
		if seg == ".." {
			// 关键：向上消化，而不是丢弃。
			// 丢弃会把 /data/../../etc 变成 /data/etc（实际指向错了位置）。
			if len(segs) > 0 && segs[len(segs)-1] != ".." {
				segs = segs[:len(segs)-1]
			} else if !absolute {
				// 相对路径可以合法地以 .. 开头，保留
				segs = append(segs, seg)
			}
			continue
		}
		segs = append(segs, seg)
	}

	if len(segs) == 0 {
		if absolute {
			return "/", nil
		}
		return "", deployErrf("远端目录无效: %q", remoteDir)
	}

	prefix := ""
	if absolute {
		prefix = "/"
	}
	return prefix + strings.Join(segs, "/"), nil
}

// ExpandRemoteHome 把 `~/xxx` 解析成真实的远端家目录。
//
// SFTP 协议本身不展开 `~`：不解析的话 `~/app` 会被当成普通路径段，
// 建出 `/~` 这种目录，而 shell 侧的重启命令却真的展开到家目录，
// 两边指向完全不同的位置。
func ExpandRemoteHome(sftp *sftp.Client, p string) (string, error) {
	if !strings.HasPrefix(p, "~") {
		return p, nil
	}
	if strings.SplitN(p, "/", 2)[0] != "~" {
		return "", deployErrf(
			"远端目录不支持 ~其他用户 的写法: %q（SFTP 无法确定该用户的家目录，请改用绝对路径）",
			p)
	}

	home, err := sftp.RealPath(".")
	if err != nil {
		return "", deployErrf(
			"无法解析远端家目录，路径 %q 请改写为绝对路径（%v）", p, err)
	}

	base := strings.TrimRight(home, "/")
	rest := strings.TrimLeft(p[1:], "/")
	if rest == "" {
		if base == "" {
			return "/", nil
		}
		return base, nil
	}
	return base + "/" + rest, nil
}

// ValidateBackupFilename 校验回滚弹窗选中的备份文件名。
//
// 备份名必须属于本服务，且不允许携带路径分隔符 —— 这是回滚路径上
// 唯一一处由用户（或界面）直接提供、随后进入远端 cp 命令的字符串，
// 放行 `..` 就等于把回滚变成任意文件写。
//
// 与 Python rollback_service 中的三段校验一致。
func ValidateBackupFilename(remoteName, backupFilename string) error {
	if remoteName == "" {
		return deployErrf("服务未指定 remote_name，无法回滚")
	}
	if strings.ContainsAny(backupFilename, `/\`) ||
		backupFilename == "." || backupFilename == ".." {
		return deployErrf("非法的备份文件名: %q，已拒绝回滚", backupFilename)
	}
	prefix := remoteName + ".bak."
	if !strings.HasPrefix(backupFilename, prefix) {
		return deployErrf(
			"备份文件名校验失败: %q 不是该服务的备份（应以 %s 开头），已拒绝回滚",
			backupFilename, prefix)
	}
	return nil
}
