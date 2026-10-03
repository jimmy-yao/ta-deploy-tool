package deployer

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/pkg/sftp"
	"golang.org/x/crypto/ssh"
)

// Conn 一次 SSH 连接（对应 Python 的 async with asyncssh.connect()）。
type Conn struct {
	ssh  *ssh.Client
	srv  config.ServerConfig
	opts Options
}

// ============================================================
// 认证
// ============================================================

// buildAuth 组装认证方式（key 优先，其次密码）。
func buildAuth(srv config.ServerConfig) ([]ssh.AuthMethod, error) {
	var methods []ssh.AuthMethod

	if srv.Key != "" {
		keyPath := config.ExpandPath(srv.Key)
		key, err := os.ReadFile(keyPath)
		if err != nil {
			return nil, deployErrf("读取私钥失败 %s: %v", keyPath, err)
		}
		signer, err := ssh.ParsePrivateKey(key)
		if err != nil {
			var pmErr *ssh.PassphraseMissingError
			if errors.As(err, &pmErr) && srv.Password != "" {
				// 口令保护的私钥：复用 password 字段作为私钥口令
				s2, pErr := ssh.ParsePrivateKeyWithPassphrase(key, []byte(srv.Password))
				if pErr != nil {
					return nil, deployErrf("解析加密私钥失败 %s: %v", keyPath, pErr)
				}
				signer = s2
			} else {
				return nil, deployErrf("解析私钥失败 %s: %v", keyPath, err)
			}
		}
		methods = append(methods, ssh.PublicKeys(signer))
	}

	if srv.Password != "" {
		methods = append(methods, ssh.Password(srv.Password))
	}

	if len(methods) == 0 {
		return nil, deployErrf("服务器 %s (%s) 未配置 key 或 password", srv.Name, srv.Host)
	}
	return methods, nil
}

// hostKeyCallback 构造主机公钥校验回调。
// 默认与原 Python known_hosts=None 一致（不校验）；
// 设置 Options.StrictHostKey 后改为严格校验。
func hostKeyCallback(opts Options) (ssh.HostKeyCallback, error) {
	if !opts.StrictHostKey {
		return ssh.InsecureIgnoreHostKey(), nil
	}
	path := opts.KnownHosts
	if path == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return nil, deployErrf("无法定位 home 目录: %v", err)
		}
		path = home + "/.ssh/known_hosts"
	}
	cb, err := knownHostsCallback(path)
	if err != nil {
		return nil, err
	}
	return cb, nil
}

// ============================================================
// 连接
// ============================================================

// Dial 建立 SSH 连接。调用方负责 Close。
func Dial(ctx context.Context, srv config.ServerConfig, opts Options) (*Conn, error) {
	if srv.Host == "" {
		return nil, deployErrf("服务器 %s 未配置 host", srv.Name)
	}
	port := srv.Port
	if port == 0 {
		port = 22
	}

	auth, err := buildAuth(srv)
	if err != nil {
		return nil, err
	}
	hk, err := hostKeyCallback(opts)
	if err != nil {
		return nil, err
	}

	addr := net.JoinHostPort(srv.Host, strconv.Itoa(port))
	sshCfg := &ssh.ClientConfig{
		User:            srv.User,
		Auth:            auth,
		HostKeyCallback: hk,
		Timeout:         opts.timeout(),
	}

	d := net.Dialer{Timeout: opts.timeout()}
	raw, err := d.DialContext(ctx, "tcp", addr)
	if err != nil {
		return nil, deployErrf("SSH 连接失败 %s: %v", addr, err)
	}

	// 握手同样受超时约束，完成后解除
	_ = raw.SetDeadline(time.Now().Add(opts.timeout()))
	clientConn, chans, reqs, err := ssh.NewClientConn(raw, addr, sshCfg)
	if err != nil {
		_ = raw.Close()
		return nil, translateDialError(err, srv, port)
	}
	_ = raw.SetDeadline(time.Time{})

	return &Conn{ssh: ssh.NewClient(clientConn, chans, reqs), srv: srv, opts: opts}, nil
}

// translateDialError 把握手错误翻译成与 Python 一致的提示。
func translateDialError(err error, srv config.ServerConfig, port int) error {
	msg := err.Error()
	if strings.Contains(msg, "unable to authenticate") ||
		strings.Contains(msg, "no supported methods remain") ||
		strings.Contains(msg, "permission denied") {
		authMode := "password"
		if srv.Key != "" {
			authMode = "key"
		}
		return deployErrf("SSH 认证失败: user=%s host=%s:%d auth=%s 原因=%v",
			srv.User, srv.Host, port, authMode, err)
	}
	return deployErrf("SSH 握手失败 %s:%d: %v", srv.Host, port, err)
}

// Close 关闭连接。
func (c *Conn) Close() error {
	if c == nil || c.ssh == nil {
		return nil
	}
	return c.ssh.Close()
}

// ============================================================
// 远程命令
// ============================================================

// safeLog 给日志回调加锁：SSH 的 stdout/stderr 各由一个 goroutine 写入。
func safeLog(log LogFn) LogFn {
	if log == nil {
		return func(string) {}
	}
	var mu sync.Mutex
	return func(s string) {
		mu.Lock()
		defer mu.Unlock()
		log(s)
	}
}

// lineWriter 按行切分写入内容并回调。
type lineWriter struct {
	mu     sync.Mutex
	buf    []byte
	prefix string
	emit   LogFn
}

func (w *lineWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.buf = append(w.buf, p...)
	for {
		idx := bytes.IndexByte(w.buf, '\n')
		if idx < 0 {
			break
		}
		line := strings.TrimRight(string(w.buf[:idx]), "\r")
		w.buf = w.buf[idx+1:]
		if line != "" {
			w.emit(w.prefix + line)
		}
	}
	return len(p), nil
}

func (w *lineWriter) flush() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if line := strings.TrimRight(string(w.buf), "\r"); line != "" {
		w.emit(w.prefix + line)
	}
	w.buf = nil
}

// Run 执行远程命令并记录输出，退出码非 0 返回 *DeployExecuteError。
// stdout/stderr 均实时逐行输出，stderr 行带 "[stderr] " 前缀。
func (c *Conn) Run(ctx context.Context, cmd string, log LogFn) error {
	log = safeLog(log)
	log("$ " + cmd)

	session, err := c.ssh.NewSession()
	if err != nil {
		return deployErrf("创建 SSH 会话失败: %v", err)
	}
	defer session.Close()

	outW := &lineWriter{emit: log}
	errW := &lineWriter{prefix: "[stderr] ", emit: log}
	session.Stdout = outW
	session.Stderr = errW

	waitErr := make(chan error, 1)
	if err := session.Start(cmd); err != nil {
		return deployErrf("启动远程命令失败: %v", err)
	}
	go func() { waitErr <- session.Wait() }()

	select {
	case <-ctx.Done():
		_ = session.Close()
		<-waitErr
		outW.flush()
		errW.flush()
		return ctx.Err()
	case we := <-waitErr:
		outW.flush()
		errW.flush()
		if we == nil {
			return nil
		}
		var ee *ssh.ExitError
		if errors.As(we, &ee) {
			return deployErrf("远程命令失败，退出码 %d: %s", ee.ExitStatus(), cmd)
		}
		return deployErrf("远程命令执行异常: %v", we)
	}
}

// Stream 启动远程命令并把输出逐行交给 onLine，直到命令结束或 ctx 取消。
// 命令退出码非 0 同样返回 *DeployExecuteError。
// 用于 `tail -f` 这类长驻命令：ctx 取消即断开。
func (c *Conn) Stream(ctx context.Context, cmd string, onLine func(string)) error {
	emit := safeLog(onLine)
	session, err := c.ssh.NewSession()
	if err != nil {
		return deployErrf("创建 SSH 会话失败: %v", err)
	}
	defer session.Close()

	outW := &lineWriter{emit: emit}
	session.Stdout = outW
	session.Stderr = outW

	if err := session.Start(cmd); err != nil {
		return deployErrf("启动远程命令失败: %v", err)
	}

	done := make(chan error, 1)
	go func() { done <- session.Wait() }()

	select {
	case <-ctx.Done():
		_ = session.Close()
		<-done
		outW.flush()
		return ctx.Err()
	case we := <-done:
		outW.flush()
		if we == nil {
			return nil
		}
		var ee *ssh.ExitError
		if errors.As(we, &ee) {
			return deployErrf("远程命令失败，退出码 %d: %s", ee.ExitStatus(), cmd)
		}
		return deployErrf("远程命令执行异常: %v", we)
	}
}

// ============================================================
// SFTP
// ============================================================

// SFTP 新建 SFTP 客户端（调用方负责 Close）。
func (c *Conn) SFTP() (*sftp.Client, error) {
	sc, err := sftp.NewClient(c.ssh)
	if err != nil {
		return nil, deployErrf("创建 SFTP 会话失败: %v", err)
	}
	return sc, nil
}

// Addr 返回 host:port，用于日志。
func (c *Conn) Addr() string {
	port := c.srv.Port
	if port == 0 {
		port = 22
	}
	return fmt.Sprintf("%s:%d", c.srv.Host, port)
}
