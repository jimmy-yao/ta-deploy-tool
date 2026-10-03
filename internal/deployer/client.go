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

	// 握手与认证同样受超时约束，完成后解除。
	// 用单独的 loginTimeout 而非建连超时：TCP 秒接但握手被拖慢是
	// 真实存在的故障形态，共用一个值会把"认证慢"报成"连不上"。
	_ = raw.SetDeadline(time.Now().Add(opts.loginTimeout()))
	clientConn, chans, reqs, err := ssh.NewClientConn(raw, addr, sshCfg)
	if err != nil {
		_ = raw.Close()
		return nil, translateDialError(err, srv, port)
	}
	// 清除握手 deadline，否则连接在建立 loginTimeout 后会被内核判定失效，
	// 短命任务看起来正常，长任务（部署 + 重启）则会在中途诡异断开。
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

// remoteTermGrace 中断命令后等待服务端回收子进程的上限。
const remoteTermGrace = 2 * time.Second

// terminateCommand 终止远端命令并有限等待会话收尾。
//
// 三步缺一不可：
//  1. SIGKILL —— 让服务端真正回收子进程。只关会话通道时服务端未必杀进程，
//     等待方会一直挂到命令自然结束，"中断"退化成空约束；
//  2. 关本地会话；
//  3. 限时等待 —— 服务端不支持 signal 时，不能让中断本身变成新的卡点。
func terminateCommand(session *ssh.Session, waitCh chan error) {
	_ = session.Signal(ssh.SIGKILL)
	_ = session.Close()
	select {
	case <-waitCh:
	case <-time.After(remoteTermGrace):
	}
}

// lineWriter 按行切分写入内容并回调。
type lineWriter struct {
	mu      sync.Mutex
	buf     []byte
	prefix  string
	emit    LogFn
	stopped bool
}

func (w *lineWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.stopped {
		// 已经向调用方报告完本次操作的结局，之后再到达的输出
		// 不能补进来——否则超时/取消返回后还会继续往日志里灌内容。
		return len(p), nil
	}
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

// stop 冲掉末行、然后封闭写入端：后续 Write 直接丢弃。
func (w *lineWriter) stop() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if line := strings.TrimRight(string(w.buf), "\r"); line != "" {
		w.emit(w.prefix + line)
	}
	w.buf = nil
	w.stopped = true
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

	// 单条命令的执行上限：远端重启脚本卡死时，本端必须给出明确的
	// 超时结论并断开会话，否则部署会永远停在"重启中"，操作者无从判断。
	runCtx, cancel := context.WithTimeout(ctx, c.opts.cmdTimeout())
	defer cancel()

	select {
	case <-runCtx.Done():
		// 远端进程必须显式终止：只关本地会话的话，服务端未必立刻回收
		// 子进程，<waitErr> 会一直等到命令自然结束，超时就退化成空约束
		// （表现为"中断耗时 = 命令总时长"）。
		terminateCommand(session, waitErr)
		outW.stop()
		errW.stop()
		// 上游主动取消（用户点停止 / 外层整体超时）优先，保持原有语义
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return deployErrf("远程命令超时（超过 %ds 未退出，已中断）: %s",
			int(c.opts.cmdTimeout()/time.Second), cmd)
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
		// tail -f 不会因为通道关闭而自行退出，不杀进程的话
		// <-done 会挂到命令自然结束，"停止监听"变成假返回。
		terminateCommand(session, done)
		outW.stop()
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
