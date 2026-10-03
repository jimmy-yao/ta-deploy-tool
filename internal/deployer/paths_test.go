package deployer

import (
	"testing"
)

// ---------- NormalizeRemoteDir ----------

func TestNormalizeRemoteDir(t *testing.T) {
	cases := []struct {
		in      string
		want    string
		wantErr bool
	}{
		// 折叠重复斜杠、丢弃 '.'
		{"/data//app/./x", "/data/app/x", false},
		{"/data/app/", "/data/app", false},

		// 关键语义：.. 是"向上消化"，不是"丢弃"。
		// 丢弃会把 /data/../../etc 错算成 /data/etc。
		{"/data/../../etc", "/etc", false},
		{"/a/b/../../../c", "/c", false},
		{"/../etc", "/etc", false}, // 不能越过根

		// 相对路径保留开头的 ..
		{"../../app", "../../app", false},
		// app 被第一个 .. 消化，剩下两个 .. 因为相对路径且前面已空而保留
		{"app/../../..", "../..", false},
		{"app/../x", "x", false},

		// 保留绝对 / 相对语义
		{"/opt/app", "/opt/app", false},
		{"app", "app", false},
		{"~/app", "~/app", false},

		// 反斜杠也当分隔符，避免 Windows 风格路径原样落到远端
		{`C:\data\app`, "C:/data/app", false},
		{`/data\app`, "/data/app", false},
		{`..\data`, "../data", false},

		// 两端空白剥掉
		{"  /opt/app  ", "/opt/app", false},

		// 根
		{"/", "/", false},

		// 空路径必须报错，绝不落到 mkdir /
		{"", "", true},
		{"   ", "", true},
		{".", "", true}, // 归一后只剩相对空路径
		{"././.", "", true},
	}

	for _, c := range cases {
		got, err := NormalizeRemoteDir(c.in)
		if c.wantErr {
			if err == nil {
				t.Errorf("NormalizeRemoteDir(%q) 应报错，实际得到 %q", c.in, got)
			}
			continue
		}
		if err != nil {
			t.Errorf("NormalizeRemoteDir(%q) 意外报错: %v", c.in, err)
			continue
		}
		if got != c.want {
			t.Errorf("NormalizeRemoteDir(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestNormalizeRemoteDirConquersDotDot 专门锁住 ".." 向上消化的语义，
// 防止回退成"直接丢弃 .."这种看似无害、实则把路径指到别处的写法。
func TestNormalizeRemoteDirConquersDotDot(t *testing.T) {
	got, err := NormalizeRemoteDir("/data/deploy/../../../etc/passwd")
	if err != nil {
		t.Fatal(err)
	}
	if got != "/etc/passwd" {
		t.Fatalf("应向上消化到 /etc/passwd, 得到 %q（丢弃 .. 会得到 /data/deploy/etc/passwd）", got)
	}
}

// ---------- ValidateBackupFilename ----------

func TestValidateBackupFilename(t *testing.T) {
	const rn = "app.jar"
	okName := "app.jar.bak.20260101_120000.000"

	if err := ValidateBackupFilename(rn, okName); err != nil {
		t.Errorf("合法备份名应通过: %v", err)
	}

	bad := []struct {
		name string
		fn   string
	}{
		{"路径穿越 ..", "../../etc/passwd"},
		{"正斜杠", "/etc/passwd"},
		{"反斜杠", `..\..\windows\system32`},
		{"当前目录", "."},
		{"上级目录", ".."},
		{"前缀不符（他人备份）", "other.jar.bak.20260101_120000.000"},
		{"前缀为空", "app.jar.tmp"},
	}
	for _, c := range bad {
		if err := ValidateBackupFilename(rn, c.fn); err == nil {
			t.Errorf("%s 应被拒绝: %q", c.name, c.fn)
		}
	}

	// remote_name 为空时必须报错，不能退化成宽松校验
	if err := ValidateBackupFilename("", okName); err == nil {
		t.Error("remote_name 为空时应报错")
	}
}

// ---------- globSafePath ----------

func TestGlobSafePath(t *testing.T) {
	ok := []string{
		"/opt/app/logs/*.log",
		"/opt/app/logs/error-?.log",
		"/opt/app/logs/[abc].log",
		"/home/user/my-app/logs/app.log",
	}
	for _, p := range ok {
		if !globSafePath.MatchString(p) {
			t.Errorf("安全通配符路径应放行: %q", p)
		}
	}

	bad := []string{
		"/opt/a b/*.log",
		"/opt/a;b/*.log",
		"/opt/$(id)/*.log",
		"/opt/a`b`/*.log",
		"/opt/a'b/*.log",
		"/opt/a\nb/*.log",
	}
	for _, p := range bad {
		if globSafePath.MatchString(p) {
			t.Errorf("含元字符的路径应拒绝: %q", p)
		}
	}
}

// ---------- filterBackups（严格前缀，不误伤其它服务） ----------

func TestFilterBackupsStrictPrefix(t *testing.T) {
	entries := []string{
		"app.jar.bak.20260101_000000.000",
		"app.jar.failed.20260101_000001.000",
		"other.jar.bak.20260101_000002.000",
		"app.jar",
		"app.jar.tmp",
	}
	got := filterBackups(entries, "app.jar.bak.")
	if len(got) != 1 || got[0] != "app.jar.bak.20260101_000000.000" {
		t.Errorf("只应命中本服务的 bak 备份, 得到 %v", got)
	}
}
