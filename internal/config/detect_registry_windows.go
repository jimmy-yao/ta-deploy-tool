//go:build windows

package config

import (
	"path/filepath"

	"golang.org/x/sys/windows/registry"
)

// registryJavaHome 从 Windows 注册表读取已安装的 JDK 根目录。
func registryJavaHome() string {
	keys := []string{
		`SOFTWARE\JavaSoft\JDK`,
		`SOFTWARE\JavaSoft\Java Development Kit`,
		`SOFTWARE\JavaSoft\Java Runtime Environment`,
	}
	for _, k := range keys {
		if home := scanRegKey(k); home != "" {
			return home
		}
	}
	return ""
}

func scanRegKey(path string) string {
	k, err := registry.OpenKey(registry.LOCAL_MACHINE, path, registry.ENUMERATE_SUB_KEYS|registry.QUERY_VALUE)
	if err != nil {
		return ""
	}
	defer k.Close()

	subs, err := k.ReadSubKeyNames(-1)
	if err != nil {
		return ""
	}
	for _, ver := range subs {
		sub, err := registry.OpenKey(k, ver, registry.QUERY_VALUE)
		if err != nil {
			continue
		}
		jh, _, err := sub.GetStringValue("JavaHome")
		sub.Close()
		if err == nil && jh != "" && fileExists(filepath.Join(jh, "bin", "java.exe")) {
			return jh
		}
	}
	return ""
}
