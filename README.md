# ChsIME++

Windows 11 中文输入法增强。

### 功能

1. 输入直角引号<br/>
   <kbd>Shift + [</kbd> = <kbd>「</kbd><br/>
   <kbd>Shift + ]</kbd> = <kbd>」</kbd>
2. 允许应用立即激活窗口而不是在任务栏闪烁<br/>

<sub>
*<sup>1.</sup> 针对 Qt 程序（如微信）改用 <code>PostMessageW</code> 以解决无法区分 <code>「</code> 与 <code>」</code> 的问题。<br/>
*<sup>2.</sup> 普通窗口通过关闭前台锁定超时处理，UAC 通过 <code>AllowConsentToStealFocus</code> 处理。<br/>
* 要对管理员权限的程序生效，需以管理员权限运行 <code>chsimexx.exe</code> 或安装自启任务。
</sub>

### 用法

- `chsimexx install` 安装
- `chsimexx start` 启动 / 重启
- `chsimexx stop` 停止
- `chsimexx uninstall` 卸载
- `chsimexx version` 显示版本
