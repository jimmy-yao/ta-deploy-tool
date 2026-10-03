package proc

import (
	"strings"
	"unicode/utf8"

	"golang.org/x/text/encoding"
	"golang.org/x/text/encoding/simplifiedchinese"
)

// LogFn 日志回调（对应 Python log(text)）。
type LogFn func(string)

// labelToEncoder 将配置里的 log_encoding 映射到 x/text 编码器。
var labelToEncoder = map[string]encoding.Encoding{
	"utf-8":   nil, // 特殊处理
	"utf8":    nil,
	"gbk":     simplifiedchinese.GBK,
	"gb18030": simplifiedchinese.GB18030,
	"gb2312":  simplifiedchinese.HZGB2312,
}

// DecodeLine 安全解码一行原始字节。
// 与 Python 版三级兜底语义一致：
//  1. 使用配置编码解码
//  2. 失败回落 GBK（忽略非法字节）
//  3. 仍失败则用替换符，绝不 panic
//
// 返回值已去除行尾 \r\n 与空白。
func DecodeLine(raw []byte, encLabel string) string {
	if len(raw) == 0 {
		return ""
	}

	if text, ok := decodeWith(raw, encLabel); ok {
		return strings.TrimRight(text, " \t\r\n")
	}

	// 回落 1：GBK（Windows 控制台常见），x/text 解码器对非法字节自动替换
	if decoded, err := simplifiedchinese.GBK.NewDecoder().Bytes(raw); err == nil && len(decoded) > 0 {
		return strings.TrimRight(string(decoded), " \t\r\n")
	}

	// 回落 2：替换非法字节，保证绝不 panic
	return strings.TrimRight(strings.ToValidUTF8(string(raw), "\uFFFD"), " \t\r\n")
}

func decodeWith(raw []byte, encLabel string) (string, bool) {
	enc, known := labelToEncoder[strings.ToLower(strings.TrimSpace(encLabel))]
	if !known {
		enc = nil
	}
	if enc == nil {
		// UTF-8（默认路径）
		if utf8.Valid(raw) {
			return string(raw), true
		}
		return "", false
	}
	decoded, err := enc.NewDecoder().Bytes(raw)
	if err != nil {
		return "", false
	}
	if !utf8.Valid(decoded) {
		return "", false
	}
	return string(decoded), true
}

// safeDecode 是 DecodeLine 的别名，便于在主流程中表达意图。
func safeDecode(raw []byte, encLabel string) string { return DecodeLine(raw, encLabel) }
