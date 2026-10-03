// Package builder 对应原 Python legacy-python/src/core/builder.py：
// Maven 异步构建、构建产物挑选、Git 更新编排、进程树清理。
package builder

import (
	"os"
	"path/filepath"
	"strings"
)

// jar 被排除的产物后缀（源码包 / 文档包 / 测试包 / maven 备份包）。
var jarExcludes = []string{
	"-sources.jar",
	"-javadoc.jar",
	"-tests.jar",
	".original",
}

// 可执行 Spring Boot fat jar 的偏好关键词。
var preferKeywords = []string{
	"service", "server", "app", "boot", "web", "main", "exec", "application",
}

// 应当避开的库/接口包关键词。
var avoidKeywords = []string{
	"api", "common", "sdk", "client", "core", "proto", "model", "dto",
}

// PickDeployableJar 从候选 jar 列表中挑选出真正可部署的 fat jar。
// 与原 Python _pick_deployable_jar 的三轮筛选语义完全一致：
//  1. 过滤 sources / javadoc / tests / original
//  2. 服务名（忽略大小写、下划线）模糊匹配，倒序优先
//  3. 关键词权重（偏好 service/server/app，避开 api/common）
func PickDeployableJar(svcName string, jars []string) string {
	if len(jars) == 0 {
		return ""
	}

	filtered := make([]string, 0, len(jars))
	for _, j := range jars {
		base := strings.ToLower(filepath.Base(j))
		excluded := false
		for _, ex := range jarExcludes {
			if strings.HasSuffix(base, ex) {
				excluded = true
				break
			}
		}
		if !excluded {
			filtered = append(filtered, j)
		}
	}
	if len(filtered) == 0 {
		return ""
	}

	svcKey := normalize(svcName)

	// 第一轮：服务名精确模糊匹配（倒序 = 最新产物优先）
	for i := len(filtered) - 1; i >= 0; i-- {
		base := normalize(filepath.Base(filtered[i]))
		if strings.Contains(base, svcKey) || strings.Contains(svcKey, base) {
			return filtered[i]
		}
	}

	// 第二轮：偏好关键词命中且不含回避关键词
	for i := len(filtered) - 1; i >= 0; i-- {
		base := strings.ToLower(filepath.Base(filtered[i]))
		if containsAny(base, preferKeywords) && !containsAny(base, avoidKeywords) {
			return filtered[i]
		}
	}

	// 第三轮：仅偏好关键词
	for i := len(filtered) - 1; i >= 0; i-- {
		base := strings.ToLower(filepath.Base(filtered[i]))
		if containsAny(base, preferKeywords) {
			return filtered[i]
		}
	}

	return filtered[len(filtered)-1]
}

// normalize 统一小写并将下划线转为中划线，用于模糊比对。
func normalize(s string) string {
	return strings.ReplaceAll(strings.ToLower(s), "_", "-")
}

func containsAny(s string, keys []string) bool {
	for _, k := range keys {
		if strings.Contains(s, k) {
			return true
		}
	}
	return false
}

// StripJarMarkers 从 Maven 输出行中提取 "Building jar:" 标记后的路径。
// 返回空串表示该行不是构建产物输出。
func StripJarMarkers(line string) string {
	const marker = "Building jar:"
	idx := strings.Index(line, marker)
	if idx < 0 {
		return ""
	}
	jarPath := strings.TrimSpace(line[idx+len(marker):])
	if !strings.HasSuffix(jarPath, ".jar") {
		return ""
	}
	return jarPath
}

// JarExists 报告路径是否存在且为普通文件。
func JarExists(p string) bool {
	if p == "" {
		return false
	}
	st, err := os.Stat(p)
	return err == nil && !st.IsDir()
}
