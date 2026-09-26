# exceptions.py

class DeployError(Exception):
    """部署工具基础业务/流程异常（已知错误，UI 日志不需要打印 Python 堆栈 Traceback）"""
    pass


class GitError(DeployError):
    """Git 相关操作异常"""
    pass


class BuildError(DeployError):
    """Maven/构建相关操作异常"""
    pass


class DeployExecuteError(DeployError):
    """部署/SSH/远程命令相关操作异常"""
    pass
