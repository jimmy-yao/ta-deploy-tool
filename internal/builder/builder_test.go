package builder

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
)

// ---------- 1. Jar 挑选算法（纯函数） ----------

func TestPickDeployableJar(t *testing.T) {
	cases := []struct {
		name string
		svc  string
		jars []string
		want string
	}{
		{
			"服务名匹配优先",
			"user-service",
			[]string{"t/common-api-1.0.jar", "t/user-service-1.0.jar"},
			"t/user-service-1.0.jar",
		},
		{
			"排除 sources/javadoc/tests/original",
			"user-service",
			[]string{
				"t/user-service-1.0-sources.jar",
				"t/user-service-1.0-javadoc.jar",
				"t/user-service-1.0-tests.jar",
				"t/user-service-1.0.original",
				"t/user-service-1.0.jar",
			},
			"t/user-service-1.0.jar",
		},
		{
			"下划线可匹配中划线",
			"user-service",
			[]string{"t/user_service-1.0.jar"},
			"t/user_service-1.0.jar",
		},
		{
			"兜底偏好 app 关键词",
			"unknown",
			[]string{"t/my-core-1.0.jar", "t/my-app-1.0.jar"},
			"t/my-app-1.0.jar",
		},
		{
			"避开 common 关键词",
			"order",
			[]string{"t/order-common-1.0.jar", "t/order-service-1.0.jar"},
			"t/order-service-1.0.jar",
		},
		{"空列表", "svc", nil, ""},
		{"全部被过滤", "svc", []string{"a-sources.jar", "b.original"}, ""},
	}

	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := PickDeployableJar(c.svc, c.jars)
			if got != c.want {
				t.Errorf("PickDeployableJar(%q, %v) = %q, 期望 %q", c.svc, c.jars, got, c.want)
			}
		})
	}
}

func TestPickDeployableJarReturnsExistingCandidate(t *testing.T) {
	// 挑出的结果必须来自输入集合
	jars := []string{"a.jar", "b-service.jar", "c.jar"}
	got := PickDeployableJar("x-service", jars)
	found := false
	for _, j := range jars {
		if j == got {
			found = true
		}
	}
	if !found {
		t.Errorf("返回了输入集合之外的结果: %q", got)
	}
}

// ---------- 2. Maven 输出标记提取 ----------

func TestStripJarMarkers(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"[INFO] Building jar: /p/target/a.jar", "/p/target/a.jar"},
		{"Building jar: target/a.jar", "target/a.jar"},
		{"[INFO] Building jar: /p/target/a.txt", ""},
		{"[INFO] BUILD SUCCESS", ""},
		{"Building jar:", ""},
		{"", ""},
	}
	for _, c := range cases {
		if got := StripJarMarkers(c.in); got != c.want {
			t.Errorf("StripJarMarkers(%q) = %q, 期望 %q", c.in, got, c.want)
		}
	}
}

// ---------- 5. 构建主流程 ----------

func TestBuildServiceMissingProjectDir(t *testing.T) {
	_, err := BuildService(context.Background(), BuildRequest{
		Maven: "mvn", ServiceName: "svc",
		ProjectDir: "/no/such/dir",
	}, nil, nil)
	var be *BuildError
	if !errors.As(err, &be) {
		t.Errorf("期望 *BuildError, 得到 %T: %v", err, err)
	}
}

func TestBuildServiceEmptyMaven(t *testing.T) {
	dir := t.TempDir()
	_, err := BuildService(context.Background(), BuildRequest{
		ServiceName: "svc", ProjectDir: dir,
	}, nil, nil)
	var be *BuildError
	if !errors.As(err, &be) {
		t.Errorf("期望 *BuildError, 得到 %v", err)
	}
}

func TestBuildServiceSuccessWithJarDetection(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	content := "#!/bin/sh\necho '[INFO] Building jar: /tmp/p/target/user-service-1.0.jar'\necho '[INFO] BUILD SUCCESS'\nexit 0\n"
	if err := os.WriteFile(script, []byte(content), 0o755); err != nil {
		t.Fatal(err)
	}

	var lines []string
	jar, err := BuildService(context.Background(), BuildRequest{
		Maven:       script,
		ServiceName: "user-service",
		ProjectDir:  dir,
		BuildArgs:   "clean package",
		Encoding:    "utf-8",
	}, func(s string) { lines = append(lines, s) }, nil)

	if err != nil {
		t.Fatalf("构建失败: %v", err)
	}
	if jar != "/tmp/p/target/user-service-1.0.jar" {
		t.Errorf("jar 路径错误: %q", jar)
	}
	if !strings.Contains(strings.Join(lines, "\n"), "BUILD SUCCESS") {
		t.Errorf("日志未捕获 BUILD SUCCESS: %v", lines)
	}
}

func TestBuildServiceFailureReturnsError(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\necho '[ERROR] COMPILATION ERROR'\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	_, err := BuildService(context.Background(), BuildRequest{
		Maven: script, ServiceName: "svc", ProjectDir: dir,
		BuildArgs: "package", Encoding: "utf-8",
	}, nil, nil)

	var be *BuildError
	if !errors.As(err, &be) {
		t.Errorf("期望 *BuildError, 得到 %v", err)
	} else if !strings.Contains(be.Error(), "svc") {
		t.Errorf("错误信息应包含服务名: %s", be.Error())
	}
}

func TestBuildServiceNoJarReturnsEmpty(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\necho '[INFO] BUILD SUCCESS'\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	jar, err := BuildService(context.Background(), BuildRequest{
		Maven: script, ServiceName: "svc", ProjectDir: dir,
		BuildArgs: "package", Encoding: "utf-8",
	}, nil, nil)
	if err != nil {
		t.Fatalf("无产物不应报错: %v", err)
	}
	if jar != "" {
		t.Errorf("期望空字符串: %q", jar)
	}
}

func TestBuildServiceCancelled(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\necho start\nsleep 30\necho done\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	var times atomic.Int32
	_, err := BuildService(context.Background(), BuildRequest{
		Maven: script, ServiceName: "svc", ProjectDir: dir,
		BuildArgs: "package", Encoding: "utf-8",
	}, nil, func() bool {
		return times.Add(1) >= 2
	})
	if !errors.Is(err, ErrCancelled) {
		t.Errorf("期望 ErrCancelled, 得到 %v", err)
	}
}

// ---------- 6. JAVA_HOME 注入 ----------

func TestBuildEnvInjectsJavaHome(t *testing.T) {
	t.Setenv("JAVA_HOME", "/old/jdk")
	t.Setenv("PATH", "/usr/bin")
	t.Setenv("FOO", "bar")

	env := buildEnv("/new/jdk-17")
	m := envMap(env)

	if m["JAVA_HOME"] != "/new/jdk-17" {
		t.Errorf("JAVA_HOME 应被覆盖: %q", m["JAVA_HOME"])
	}
	if !strings.HasPrefix(m["PATH"], "/new/jdk-17/bin") {
		t.Errorf("JAVA_HOME/bin 应前置到 PATH: %q", m["PATH"])
	}
	if m["FOO"] != "bar" {
		t.Errorf("其他环境变量应保留")
	}
}

func TestBuildEnvEmptyJavaHomeKeepsSystem(t *testing.T) {
	t.Setenv("JAVA_HOME", "/sys/jdk")
	env := buildEnv("")
	m := envMap(env)
	if m["JAVA_HOME"] != "/sys/jdk" {
		t.Errorf("未配置时应保留系统 JAVA_HOME: %q", m["JAVA_HOME"])
	}
}

// ---------- helpers ----------

func envMap(env []string) map[string]string {
	m := map[string]string{}
	for _, kv := range env {
		if i := strings.Index(kv, "="); i > 0 {
			m[kv[:i]] = kv[i+1:]
		}
	}
	return m
}
