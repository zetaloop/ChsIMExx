# ChsIME++

Windows 11 中文输入法增强。

### 功能

中文输入法中文模式时：

1. 直角引号<br/>
   <kbd>Shift + [</kbd> = <kbd>「</kbd><br/>
   <kbd>Shift + ]</kbd> = <kbd>」</kbd>

<sub>
*<sup>1.</sup> 针对 Qt 程序（如微信）改用 <code>PostMessageW</code> 以解决无法区分 <code>「</code> 与 <code>」</code> 的问题。<br/>
* 要对管理员权限的程序生效，需以管理员权限运行 <code>chsimexx.exe</code>。
</sub>

### 用法

构建 chsimexx.exe，扔到随便哪儿，设为开机自启。

- `chsimexx` 启动 / 重启
- `chsimexx stop` 停止
