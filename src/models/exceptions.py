# exceptions.py
class DeployError(Exception):
    """部署工具业务/流程已知异常（UI 日志中不需要打 Python 追溯堆栈 Traceback）。"""
    pass


class BuildError(DeployError):
    """构建相关异常（Maven 编译失败等）。"""
    pass


class GitError(DeployError):
    """Git 操作异常。"""
    pass


class ConfigError(DeployError):
    """配置错误（循环依赖、选中了不存在的服务等）。"""
    pass


class DeployExecuteError(DeployError):
    """部署执行阶段异常（SSH/SFTP 失败、本地 jar 不存在等）。"""
    pass
