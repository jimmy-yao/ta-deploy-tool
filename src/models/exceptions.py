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


class ServiceCancelled(DeployError):
    """任务里的**某一个服务**被用户单独取消。

    与整任务取消（``asyncio.CancelledError``）刻意分开：整任务取消要中止
    ``run()`` 并抛出去，单服务取消只是「跳过这一个、其余照跑」，所以它是
    可以被正常 except 捕获、也不该记成构建/部署失败的普通异常。
    """

    def __init__(self, name: str):
        super().__init__(f"{name} 已被用户取消")
        self.name = name
