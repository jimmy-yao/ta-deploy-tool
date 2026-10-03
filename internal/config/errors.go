package config

import (
	"fmt"
	"strings"
)

// ServerError 服务引用了未定义的 server。
type ServerError struct {
	Service string
	Server  string
	Known   []string
}

func (e *ServerError) Error() string {
	return fmt.Sprintf("服务 %s 引用了未定义的 server: %s。已定义: [%s]",
		e.Service, e.Server, strings.Join(e.Known, ", "))
}

// LoadError 配置文件读取/解析错误。
type LoadError struct {
	Path string
	Err  error
}

func (e *LoadError) Error() string {
	return fmt.Sprintf("加载配置文件 %s 失败: %v", e.Path, e.Err)
}

func (e *LoadError) Unwrap() error { return e.Err }
