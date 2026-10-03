package config

import (
	"regexp"
	"strings"
)

// ============================================================
// 远端路径的 shell 转义
//
// 位置与 Python 对齐：基准分支把 quote_remote_path 放在
// src/models/config.py，resolve_restart_cmd 直接调用它。
// 放在 config 而非 deployer，deployer 才能把同一套转义用于
// tail / cp / cd，避免出现"重启命令转义了、回滚命令没转义"。
// ============================================================

// shellSafeWord 与 Python shlex.quote 的安全字符集一致：
// r'[^\w@%+=:,./-]'（re.ASCII）。命中则原样返回，不加引号。
var shellSafeWord = regexp.MustCompile(`^[A-Za-z0-9_@%+=:,./-]+$`)

// QuoteRemotePath 把远端路径安全地嵌进 POSIX shell 命令。
//
// 按 '/' 切段、逐段 shlex.quote，再用**未加引号**的 '/' 拼回去：
//   - 空格、引号、`;` `&` 等元字符一律被转义，命令不会被截断或拼成多条
//   - 首段的空串（绝对路径）与 `~` / `~user` 保持裸露，`cd ~/app` 的家目录
//     展开照常生效（整体加引号会把它变成字面量 `~`）
//   - 不需要转义时输出与原文一致，日志里的命令依旧可读
//
// 与 Python quote_remote_path 一致。
func QuoteRemotePath(path string) string {
	normalized := strings.ReplaceAll(path, "\\", "/")
	segs := strings.Split(normalized, "/")
	for i, seg := range segs {
		if i == 0 && (seg == "" || strings.HasPrefix(seg, "~")) {
			continue
		}
		segs[i] = QuoteShellWord(seg)
	}
	return strings.Join(segs, "/")
}

// QuoteShellWord 对应 Python shlex.quote：
// 安全字符原样返回；空串变 ”；其余用单引号包裹并把内部 ' 转义。
func QuoteShellWord(s string) string {
	if s == "" {
		return "''"
	}
	if shellSafeWord.MatchString(s) {
		return s
	}
	return "'" + strings.ReplaceAll(s, "'", `'"'"'`) + "'"
}
