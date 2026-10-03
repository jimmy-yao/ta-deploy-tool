package config

import (
	"strings"
	"testing"
)

// ---------- QuoteRemotePath ----------

func TestQuoteRemotePath(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		// 安全字符原样返回，日志里的命令依旧可读
		{"/opt/app/x.jar", "/opt/app/x.jar"},
		{"/opt/app/My-App_1.0+rc2.jar", "/opt/app/My-App_1.0+rc2.jar"},

		// 绝对路径首段空串保持裸露；其余段含空格则单独转义
		{"/a b/c", "/'a b'/c"},
		{"/a//b", "/a/''/b"},

		// ~ 段保持裸露，cd ~/app 的家目录展开才生效
		{"~/app/x.jar", "~/app/x.jar"},
		{"~/my app/x.jar", "~/'my app'/x.jar"},
		{"~", "~"},

		// 反斜杠统一成 /
		{`C:\data\app`, "C:/data/app"},

		// 单引号转义（POSIX 写法 ' -> '"'"'）
		{"/opt/a'b/x", `/opt/'a'"'"'b'/x`},

		{"", ""},
	}

	for _, c := range cases {
		if got := QuoteRemotePath(c.in); got != c.want {
			t.Errorf("QuoteRemotePath(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestQuoteRemotePathBlocksInjection 命令拼接是最直接的注入面：
// 结果按 '/' 切段后，每一段要么是安全字符、要么是空串/`~`、
// 要么被完整单引号包裹 —— 三者之外的任何残留都意味着元字符逃逸。
func TestQuoteRemotePathBlocksInjection(t *testing.T) {
	for _, evil := range []string{
		"/opt/app/a b.jar",
		"/opt/app/a;id.jar",
		"/opt/app/a&&id.jar",
		"/opt/app/a`id`.jar",
		"/opt/app/a$(id).jar",
		"/opt/app/a|id.jar",
		"/opt/app/a\nid.jar",
		"/opt/app/a\".jar",
		"/opt/app/a'b.jar",
		"/opt/app/a&id<id.jar",
	} {
		got := QuoteRemotePath(evil)
		for _, seg := range strings.Split(got, "/") {
			if seg == "" || seg == "~" || shellSafeWord.MatchString(seg) {
				continue
			}
			if !strings.HasPrefix(seg, "'") || !strings.HasSuffix(seg, "'") {
				t.Errorf("QuoteRemotePath(%q) 的段 %q 未被正确转义: %q", evil, seg, got)
			}
		}
	}
}

// TestQuoteShellWord 覆盖 shlex.quote 的三条分支。
func TestQuoteShellWord(t *testing.T) {
	if got := QuoteShellWord("plain.txt"); got != "plain.txt" {
		t.Errorf("安全字符应原样返回: %q", got)
	}
	if got := QuoteShellWord(""); got != "''" {
		t.Errorf("空串应变 '' : %q", got)
	}
	if got := QuoteShellWord("a b"); got != "'a b'" {
		t.Errorf("空格应单引号包裹: %q", got)
	}
	if got := QuoteShellWord("a'b"); got != `'a'"'"'b'` {
		t.Errorf("内部单引号应转义: %q", got)
	}
}

// ---------- ResolveRestartCmd 的转义 ----------

// TestResolveRestartCmdQuotesPaths 默认重启命令里的两个路径都必须转义，
// 否则 remote_dir 里的空格会把 `cd a b && bash x` 截断成 `cd a` + 后续垃圾。
func TestResolveRestartCmdQuotesPaths(t *testing.T) {
	svc := &ServiceConfig{RemoteDir: "/opt/my app/logs"}
	cmd, ok := ResolveRestartCmd(svc, "bin/start.sh")
	if !ok {
		t.Fatal("应启用重启")
	}
	want := "cd /opt/'my app'/logs && bash bin/start.sh"
	if cmd != want {
		t.Errorf("重启命令未正确转义:\n got %q\nwant %q", cmd, want)
	}

	// 带引号的目录同样不能逃逸
	svc = &ServiceConfig{RemoteDir: "/opt/a'b"}
	cmd, _ = ResolveRestartCmd(svc, "start.sh")
	if strings.Contains(cmd, "/opt/a'b") {
		t.Errorf("路径中的单引号未转义: %q", cmd)
	}

	// ~ 段保持裸露，家目录展开仍然生效
	svc = &ServiceConfig{RemoteDir: "~/app"}
	cmd, _ = ResolveRestartCmd(svc, "bin/start.sh")
	if cmd != "cd ~/app && bash bin/start.sh" {
		t.Errorf("~ 路径不应被引号包住: %q", cmd)
	}
}
