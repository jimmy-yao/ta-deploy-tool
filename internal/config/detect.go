package config

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// DetectJavaHome 自动扫描系统中的 JDK 根目录。
// 顺序：JAVA_HOME 环境变量 → PATH 中的 java → 常见安装目录。
func DetectJavaHome() string {
	javaExe := "java"
	if IsWindows() {
		javaExe = "java.exe"
	}

	// 1. 环境变量
	if jh := strings.TrimSpace(os.Getenv("JAVA_HOME")); jh != "" {
		if fileExists(filepath.Join(jh, "bin", javaExe)) {
			return filepath.ToSlash(jh)
		}
	}

	// 2. PATH
	if p, err := exec.LookPath(javaExe); err == nil {
		if abs, err := filepath.Abs(p); err == nil {
			// java → <home>/bin/java
			home := filepath.Dir(filepath.Dir(abs))
			if fileExists(filepath.Join(home, "bin", javaExe)) {
				return filepath.ToSlash(home)
			}
		}
	}

	// 3. 常见安装目录
	for _, base := range javaSearchDirs() {
		if found, ok := scanForTool(base, javaExe, 3); ok {
			return filepath.ToSlash(found)
		}
	}

	// 4. Windows 注册表
	if IsWindows() {
		if home := registryJavaHome(); home != "" {
			return filepath.ToSlash(home)
		}
	}
	return ""
}

// DetectMavenPath 自动扫描系统中的 mvn 可执行文件。
// 顺序：MAVEN_HOME / M2_HOME → PATH → 常见安装目录。
func DetectMavenPath() string {
	mvnNames := []string{"mvn.cmd", "mvn.bat", "mvn"}
	if !IsWindows() {
		mvnNames = []string{"mvn"}
	}

	// 1. 环境变量
	for _, envVar := range []string{"MAVEN_HOME", "M2_HOME"} {
		if root := strings.TrimSpace(os.Getenv(envVar)); root != "" {
			for _, n := range mvnNames {
				p := filepath.Join(root, "bin", n)
				if fileExists(p) {
					return filepath.ToSlash(p)
				}
			}
		}
	}

	// 2. PATH
	for _, n := range mvnNames {
		if p, err := exec.LookPath(n); err == nil {
			if abs, err := filepath.Abs(p); err == nil {
				return filepath.ToSlash(abs)
			}
		}
	}

	// 3. 常见安装目录
	for _, base := range mavenSearchDirs() {
		if found, ok := scanForTool(base, mvnNames[0], 3); ok {
			return filepath.ToSlash(found)
		}
	}
	return ""
}

// javaSearchDirs 返回 Java 的候选根目录（按平台）。
func javaSearchDirs() []string {
	if IsWindows() {
		return []string{
			`C:\Program Files\Java`,
			`C:\Program Files (x86)\Java`,
			`C:\Program Files\Eclipse Adoptium`,
			`C:\Program Files\Zulu`,
			`C:\Program Files\Amazon Corretto`,
			`C:\Program Files\LibericaJDK`,
			`C:\Program Files\Microsoft`,
			`C:\Tools`, `D:\Tools`, `D:\Java`,
			`D:\Program Files\Java`, `D:\env`,
		}
	}
	if runtime.GOOS == "darwin" {
		return []string{"/Library/Java/JavaVirtualMachines"}
	}
	return []string{
		"/usr/lib/jvm",
		"/usr/java",
		"/opt",
	}
}

// mavenSearchDirs 返回 Maven 的候选根目录（按平台）。
func mavenSearchDirs() []string {
	if IsWindows() {
		return []string{
			`C:\apache-maven`,
			`C:\Program Files\Apache\maven`,
			`C:\Program Files\apache-maven`,
			`C:\maven`, `D:\maven`,
			`C:\Tools`, `D:\Tools`, `D:\env`, `D:\Program Files`,
			`C:\ProgramData\chocolatey\lib\maven`,
		}
	}
	return []string{"/opt", "/usr/local", "/usr/share"}
}

// scanForTool 在 base 下最多 maxDepth 层查找 toolName，返回其所在目录。
func scanForTool(base, toolName string, maxDepth int) (string, bool) {
	if base == "" || !dirExists(base) {
		return "", false
	}
	var found string
	depth := 0
	_ = filepath.WalkDir(base, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if !d.IsDir() {
			return nil
		}
		rel, relErr := filepath.Rel(base, path)
		if relErr == nil {
			depth = strings.Count(rel, string(filepath.Separator))
			if depth > maxDepth {
				return filepath.SkipDir
			}
		}
		if fileExists(filepath.Join(path, "bin", toolName)) {
			found = path
			return filepath.SkipAll
		}
		return nil
	})
	return found, found != ""
}
