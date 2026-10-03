// Package proc 提供跨平台子进程执行、编码解码与进程树清理能力。
//
// 对应原 Python 中重复实现的 _run_git / _run_git_process / _kill_process_tree /
// _decode 逻辑，统一收敛到一处，供 builder 与 gitops 复用。
package proc
