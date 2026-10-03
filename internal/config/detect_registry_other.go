//go:build !windows

package config

// registryJavaHome 仅 Windows 存在注册表，其他平台返回空。
func registryJavaHome() string { return "" }
