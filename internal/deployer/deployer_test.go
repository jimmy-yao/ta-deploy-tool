package deployer

import (
	"bytes"
	"context"
	"strings"
	"testing"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// ---------- 1. 时间戳 / 备份命名 ----------

func TestTimestampMsFormat(t *testing.T) {
	ts := timestampMs(time.Date(2026, 3, 5, 14, 7, 9, 123456789, time.Local))
	if ts != "20260305_140709.123" {
		t.Errorf("时间戳格式错误: %q", ts)
	}
}

func TestBackupName(t *testing.T) {
	got := backupName("app.jar", "20260305_140709.123")
	if got != "app.jar.bak.20260305_140709.123" {
		t.Errorf("备份名错误: %q", got)
	}
}

// ---------- 2. 远端路径 ----------

func TestRemoteJoin(t *testing.T) {
	cases := []struct{ dir, name, want string }{
		{"/opt/app", "a.jar", "/opt/app/a.jar"},
		{"/opt/app/", "a.jar", "/opt/app/a.jar"},
		{"/", "a.jar", "/a.jar"},
		{"", "a.jar", "/a.jar"},
		{"/opt//app", "a.jar", "/opt//app/a.jar"},
	}
	for _, c := range cases {
		if got := remoteJoin(c.dir, c.name); got != c.want {
			t.Errorf("remoteJoin(%q,%q)=%q want %q", c.dir, c.name, got, c.want)
		}
	}
}

func TestRemoteSegments(t *testing.T) {
	cases := []struct {
		dir  string
		abs  bool
		segs []string
	}{
		{"/opt/app/common", true, []string{"opt", "app", "common"}},
		{"opt/app", false, []string{"opt", "app"}},
		{"/", false, nil},
		{"", false, nil},
		{"///opt//app///", true, []string{"opt", "app"}},
		{"/opt/./app", true, []string{"opt", "app"}},
	}
	for _, c := range cases {
		abs, segs := remoteSegments(c.dir)
		if abs != c.abs {
			t.Errorf("remoteSegments(%q).abs=%v want %v", c.dir, abs, c.abs)
		}
		if strings.Join(segs, ",") != strings.Join(c.segs, ",") {
			t.Errorf("remoteSegments(%q).segs=%v want %v", c.dir, segs, c.segs)
		}
	}
}

// ---------- 3. 备份清理 ----------

func TestFilterBackups(t *testing.T) {
	entries := []string{
		"app.jar",
		"app.jar.bak.20260101_000000.000",
		"app.jar.bak.20260301_000000.000",
		"app.jar.bak.20260201_000000.000",
		"other.jar.bak.20260401_000000.000",
		"app.jar.tmp",
	}
	got := filterBackups(entries, "app.jar.bak.")
	want := []string{
		"app.jar.bak.20260301_000000.000",
		"app.jar.bak.20260201_000000.000",
		"app.jar.bak.20260101_000000.000",
	}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("过滤/排序错误:\n got=%v\nwant=%v", got, want)
	}
}

func TestBackupsToRemove(t *testing.T) {
	backups := []string{"b3", "b2", "b1"}

	if got := backupsToRemove(backups, 5); len(got) != 0 {
		t.Errorf("keep 大于数量时不应删除: %v", got)
	}
	if got := backupsToRemove(backups, 2); strings.Join(got, ",") != "b1" {
		t.Errorf("keep=2 应只删最旧的: %v", got)
	}
	if got := backupsToRemove(backups, 0); len(got) != 0 {
		t.Errorf("keep<=0 表示不清理: %v", got)
	}
	if got := backupsToRemove(backups, -1); len(got) != 0 {
		t.Errorf("keep<0 表示不清理: %v", got)
	}
	if got := backupsToRemove(nil, 3); len(got) != 0 {
		t.Errorf("空列表不应删除: %v", got)
	}
}

// ---------- 4. 服务名 / 日志路径 ----------

func TestRemoteNameOf(t *testing.T) {
	svc := &config.ServiceConfig{Name: "s", RemoteName: "x.jar", LocalJar: "target/y.jar"}
	if RemoteNameOf(svc) != "x.jar" {
		t.Error("显式 remote_name 优先")
	}
	svc.RemoteName = ""
	svc.LocalJar = "target/order-service-1.0.jar"
	if RemoteNameOf(svc) != "order-service-1.0.jar" {
		t.Errorf("应回退到 jar 文件名: %q", RemoteNameOf(svc))
	}
	svc.LocalJar = `C:\code\svc\target\a.jar`
	if RemoteNameOf(svc) != "a.jar" {
		t.Errorf("Windows 路径应回退到文件名: %q", RemoteNameOf(svc))
	}
	svc.LocalJar = ""
	if RemoteNameOf(svc) != "" {
		t.Errorf("无信息时应为空: %q", RemoteNameOf(svc))
	}
}

func TestResolveLogPath(t *testing.T) {
	svc := &config.ServiceConfig{Name: "s", RemoteDir: "/opt/app"}
	if got := ResolveLogPath(svc); got != "/opt/app/logs/info.log" {
		t.Errorf("默认日志路径错误: %q", got)
	}

	lp := "/var/log/app.log"
	svc.LogPath = &lp
	if got := ResolveLogPath(svc); got != "/var/log/app.log" {
		t.Errorf("绝对路径应原样返回: %q", got)
	}

	rel := "logs/custom.log"
	svc.LogPath = &rel
	if got := ResolveLogPath(svc); got != "/opt/app/logs/custom.log" {
		t.Errorf("相对路径应前置 remote_dir: %q", got)
	}

	tilde := "~/logs/x.log"
	svc.LogPath = &tilde
	if got := ResolveLogPath(svc); got != "~/logs/x.log" {
		t.Errorf("~ 路径应原样返回: %q", got)
	}
}

func TestTailCommand(t *testing.T) {
	got := TailCommand("/opt/app/logs/info.log", 200)
	if got != "tail -n 200 -f '/opt/app/logs/info.log' 2>&1" {
		t.Errorf("命令错误: %q", got)
	}
	got = TailCommand("/opt/app/logs/*.log", 100)
	if got != "tail -n 100 -f /opt/app/logs/*.log 2>&1" {
		t.Errorf("通配符不应加引号: %q", got)
	}
	got = TailCommand("/a.log", 0)
	if !strings.Contains(got, "-n 200") {
		t.Errorf("行数应兜底到 200: %q", got)
	}
}

// ---------- 5. 认证 ----------

func TestBuildAuthNoCredentials(t *testing.T) {
	_, err := buildAuth(config.ServerConfig{Name: "s", Host: "1.1.1.1"})
	if err == nil {
		t.Fatal("无凭据应报错")
	}
	if !strings.Contains(err.Error(), "未配置 key 或 password") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestBuildAuthMissingKeyFile(t *testing.T) {
	_, err := buildAuth(config.ServerConfig{
		Name: "s", Host: "1.1.1.1", Key: "/no/such/key",
	})
	if err == nil {
		t.Fatal("私钥不存在应报错")
	}
	if !strings.Contains(err.Error(), "读取私钥失败") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestBuildAuthPasswordOnly(t *testing.T) {
	m, err := buildAuth(config.ServerConfig{Name: "s", Host: "h", Password: "p"})
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if len(m) != 1 {
		t.Errorf("应只有 1 种认证方式: %d", len(m))
	}
}

func TestBuildAuthKeyAndPassword(t *testing.T) {
	keyPath := tempKeyPath(t)
	writeTestKey(t, keyPath)
	m, err := buildAuth(config.ServerConfig{Name: "s", Host: "h", Key: keyPath, Password: "x"})
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if len(m) != 2 {
		t.Errorf("应包含 key + password 两种方式: %d", len(m))
	}
}

func TestBuildAuthEncryptedKeyUsesPasswordAsPassphrase(t *testing.T) {
	keyPath := tempKeyPath(t)
	writeEncryptedTestKey(t, keyPath, "secret")

	m, err := buildAuth(config.ServerConfig{
		Name: "s", Host: "h", Key: keyPath, Password: "secret",
	})
	if err != nil {
		t.Fatalf("口令正确时应解析成功: %v", err)
	}
	if len(m) == 0 {
		t.Error("应至少产生一种认证方式")
	}

	if _, err := buildAuth(config.ServerConfig{
		Name: "s", Host: "h", Key: keyPath, Password: "wrong",
	}); err == nil {
		t.Error("口令错误时应报错")
	}
}

// ---------- 6. 主机密钥策略 ----------

func TestHostKeyCallbackDefaultInsecure(t *testing.T) {
	cb, err := hostKeyCallback(Options{})
	if err != nil {
		t.Fatalf("默认策略不应报错: %v", err)
	}
	if cb == nil {
		t.Fatal("回调不应为 nil")
	}
}

func TestHostKeyCallbackStrictRequiresKnownHosts(t *testing.T) {
	_, err := hostKeyCallback(Options{StrictHostKey: true, KnownHosts: "/no/such/known_hosts"})
	if err == nil {
		t.Fatal("严格模式缺少 known_hosts 应报错")
	}
	if !strings.Contains(err.Error(), "known_hosts") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

// ---------- 7. 进度与行输出 ----------

func TestProgressWriter(t *testing.T) {
	var buf bytes.Buffer
	var reports [][2]int64
	pw := &progressWriter{
		dst:   &buf,
		total: 10,
		cb: func(copied, total int64) {
			reports = append(reports, [2]int64{copied, total})
		},
	}
	if _, err := pw.Write([]byte("abc")); err != nil {
		t.Fatal(err)
	}
	if _, err := pw.Write([]byte("defg")); err != nil {
		t.Fatal(err)
	}
	if buf.String() != "abcdefg" {
		t.Errorf("写入错误: %q", buf.String())
	}
	if len(reports) != 2 || reports[1][0] != 7 || reports[1][1] != 10 {
		t.Errorf("进度回调错误: %v", reports)
	}
}

func TestProgressWriterCancelled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	var buf bytes.Buffer
	pw := &progressWriter{dst: &buf, total: 10, ctx: ctx}
	if _, err := pw.Write([]byte("x")); err == nil {
		t.Error("ctx 取消后应返回错误")
	}
}

func TestProgressWriterNilCallback(t *testing.T) {
	var buf bytes.Buffer
	pw := &progressWriter{dst: &buf, total: 10}
	if _, err := pw.Write([]byte("x")); err != nil {
		t.Fatal(err)
	}
}

func TestLineWriter(t *testing.T) {
	var lines []string
	w := &lineWriter{emit: func(s string) { lines = append(lines, s) }}
	w.Write([]byte("hello\nworld\r\npar"))
	w.Write([]byte("tial\n\nend\n"))
	if strings.Join(lines, "|") != "hello|world|partial|end" {
		t.Errorf("行切分错误: %v", lines)
	}
	w.flush()
	if len(lines) != 4 {
		t.Errorf("flush 不应重复输出: %v", lines)
	}
}

func TestLineWriterPrefix(t *testing.T) {
	var lines []string
	w := &lineWriter{prefix: "[stderr] ", emit: func(s string) { lines = append(lines, s) }}
	w.Write([]byte("boom\n"))
	if lines[0] != "[stderr] boom" {
		t.Errorf("前缀错误: %q", lines[0])
	}
}

func TestSafeLogNil(t *testing.T) {
	safeLog(nil)("x") // 不应 panic
	called := 0
	f := safeLog(func(string) { called++ })
	f("a")
	f("b")
	if called != 2 {
		t.Errorf("回调未透传: %d", called)
	}
}

// ---------- 8. 错误翻译 ----------

func TestDeployErrorType(t *testing.T) {
	err := deployErrf("远程命令失败，退出码 %d: %s", 1, "sh start.sh")
	var de *DeployExecuteError
	if !asDeployError(err, &de) {
		t.Fatalf("类型错误: %T", err)
	}
	if de.Error() != "远程命令失败，退出码 1: sh start.sh" {
		t.Errorf("消息错误: %q", de.Error())
	}
}

func TestTranslateDialErrorAuth(t *testing.T) {
	srv := config.ServerConfig{Name: "dev", Host: "1.1.1.1", User: "root", Password: "p"}
	err := translateDialError(errAuth(), srv, 22)
	if !strings.Contains(err.Error(), "SSH 认证失败") {
		t.Errorf("应识别为认证失败: %v", err)
	}
	if !strings.Contains(err.Error(), "auth=password") {
		t.Errorf("应标注认证方式: %v", err)
	}
	srv.Key = "/k"
	err = translateDialError(errAuth(), srv, 22)
	if !strings.Contains(err.Error(), "auth=key") {
		t.Errorf("应标注 key 认证: %v", err)
	}
}

func TestTranslateDialErrorOther(t *testing.T) {
	srv := config.ServerConfig{Name: "dev", Host: "1.1.1.1"}
	err := translateDialError(errOther(), srv, 2222)
	if strings.Contains(err.Error(), "认证失败") {
		t.Errorf("不应误判为认证失败: %v", err)
	}
	if !strings.Contains(err.Error(), "2222") {
		t.Errorf("应包含端口: %v", err)
	}
}

// ---------- 9. 空连接安全 ----------

func TestNilConnCloseSafe(t *testing.T) {
	var c *Conn
	if err := c.Close(); err != nil {
		t.Errorf("nil 连接 Close 应安全: %v", err)
	}
}

func TestServerLabel(t *testing.T) {
	if got := serverLabel(config.ServerConfig{Name: "dev", Host: "h"}); got != "dev@h:22" {
		t.Errorf("端口缺省应为 22: %q", got)
	}
	if got := serverLabel(config.ServerConfig{Name: "dev", Host: "h", Port: 2222}); got != "dev@h:2222" {
		t.Errorf("端口错误: %q", got)
	}
}

// ---------- helpers ----------

func asDeployError(err error, target **DeployExecuteError) bool {
	e, ok := err.(*DeployExecuteError)
	if ok {
		*target = e
	}
	return ok
}

type fakeErr struct{ msg string }

func (e *fakeErr) Error() string { return e.msg }

func errAuth() error  { return &fakeErr{"ssh: unable to authenticate, no supported methods remain"} }
func errOther() error { return &fakeErr{"ssh: handshake failed: connection reset"} }
