// Package deploytool 在仓库根包中嵌入默认配置模板，
// 使打包后的单个可执行文件也能在首次运行时生成 config.yaml。
package deploytool

import _ "embed"

// TemplateYAML 是 config.template.yaml 的原始内容。
//
//go:embed config.template.yaml
var TemplateYAML []byte

// AppIconPNG 是窗口图标（assets/app.png）。
//
//go:embed assets/app.png
var AppIconPNG []byte

// Version 构建时可用 -ldflags "-X github.com/jimmy-yao/ta-deploy-tool.Version=x.y.z" 覆盖。
var Version = "1.0.0-go"
