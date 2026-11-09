# RGB Tray

[English](README.md) · [Русский](README.ru.md) · **简体中文** · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md) · [日本語](README.ja.md)

一个小巧的 Windows 托盘应用，直接通过 HID 控制键盘、鼠标和灯具的 RGB 灯效（Tuya 灯泡则通过局域网控制），无需在后台运行厂商软件（G HUB、Synapse、Armoury Crate、SignalRGB）。

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="托盘面板">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="单键编辑器">
</p>

## 支持的设备

| 设备 | 连接方式 | 支持的功能 |
|---|---|---|
| Logitech G915 TKL | LIGHTSPEED 接收器 | 常亮、呼吸、光谱循环、关闭；单键颜色；电量 |
| Razer Naga V2 Pro | HyperSpeed 接收器 / 底座 | Logo 与滚轮分区独立控制；灯效；电量 |
| Razer Mouse Dock Pro | USB | 常亮、呼吸、光谱循环、关闭 |
| 另外约 90 款 Razer 鼠标、键盘和配件（Basilisk、DeathAdder、Viper、Cobra、BlackWidow、Huntsman、Ornata、Firefly、Goliathus、Base Station…），数据来自 OpenRazer — [列表](profiles) | USB / 接收器 | 常亮、呼吸、光谱循环、关闭；鼠标分区；电量。**未经测试**：设备卡片上会注明，欢迎反馈。HyperSpeed 无线键盘目前仅支持有线连接。 |
| ASUS 笔记本键盘（Aura） | 内置 | 常亮、呼吸、光谱循环、关闭；通过 LampArray 控制键盘分区和机身灯 |
| Geekboard MacroPad v2（QMK/VIA） | USB | 常亮、呼吸、光谱循环、关闭 |
| 其他支持 VIA 和 RGB 的 QMK 键盘（Keychron、客制化键盘…） | USB | 常亮、呼吸、光谱循环、关闭、亮度（呼吸和光谱循环由应用绘制） |
| 支持 Windows 动态光效标准（HID LampArray）的设备 | USB / 内置 | 常亮、呼吸、光谱循环、关闭、设备自带的分区（呼吸和光谱循环由应用绘制） |
| Tuya Wi-Fi 灯泡（Smart Life），例如 LINUS A60 | Wi-Fi，局域网 | 彩色、白光（选择接近白色的颜色时会启用白光 LED 及其色温）、呼吸、光谱循环、关闭；不会随显示器一起熄灭。需要灯泡的本地密钥，见下文 |

使用相同协议的其他型号可能也能工作，但未经测试。

### Tuya 灯泡

RGB Tray 通过局域网（协议 3.3）控制 Tuya 灯泡，需要使用灯泡的*本地密钥*（local key），而该密钥只能从 Tuya 云端获取。只需获取一次，例如使用 [tinytuya](https://github.com/jasonacox/tinytuya)（`python -m tinytuya wizard`），或 `tuya-device-sharing-sdk` 的 Smart Life 扫码登录。然后在 `%APPDATA%\rgbtray\tuya.toml` 中列出灯泡：

```toml
[[device]]
name = "台灯"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

应用会自行在局域网中找到灯泡并记住其地址（`local_ip`）。灯泡必须添加在 **Smart Life** 或 **Tuya Smart** 中：品牌定制版应用的账号无法提供密钥。灯泡同一时间只接受一个本地连接：手机上打开 Smart Life 应用时，RGB Tray 无法连接到它。

### 申请支持新设备

右键单击托盘图标 → **Request device support…**（申请设备支持）。RGB Tray 会列出你的 HID 设备，标记出可能具有可控灯效的设备，将报告复制到剪贴板，并打开[申请表单](../../issues/new?template=device-request.yml)。填写设备型号并粘贴报告即可。报告中的序列号会被截短。

Razer 或 QMK/VIA 设备，以及使用简单协议的设备，通常无需发布新版本即可立即添加：只需用一个小的配置文件描述它（[docs/profiles.md](docs/profiles.md)）。

### 借助 Claude 添加你的设备

使用 [Claude Code](https://claude.com/claude-code)，即使不懂 Rust 或 HID，你也可以自己让设备工作起来。克隆本仓库，在其中打开 Claude Code 并输入 `/add-device`。Claude 会读取设备报告，查找协议（OpenRGB、OpenRazer、厂商应用的 USB 抓包），编写配置文件或驱动，并与你一起测试：它看不到灯光，所以每一步之后都会询问设备的反应。可能损坏设备的命令（固件更新、写入闪存）一律禁止。设备正常工作后，Claude 可以将其提交为 pull request，从而内置到应用中供所有人使用。

## 功能

应用界面提供英文和俄文两种语言。

- 一个面板管理所有设备：颜色、灯效、亮度，可一次性设置全部设备，也可逐个设置。
- 全局亮度，并可在其基础上为每个设备单独调节亮度。
- 呼吸和光谱循环的速度，可全局设置，也可按设备设置。
- 在键盘的按键编辑器中，与预设并列提供键盘的固件灯效：Logitech G915 上的波浪和涟漪。给任意按键上色即可恢复按键颜色。
- Logitech 键盘的单键编辑器：画笔、取色器、彩虹、双色渐变、WASD 和仅字母键预设、高亮单词、保存配色方案、ANSI/ISO 布局。
- 支持全局快捷键的预设（包括 F13–F24 与 Ctrl/Alt/Shift/Win 组合）。
- 每个设备可设置一个全局快捷键，用于关闭设备灯光并按原样重新打开（设备卡片上名称后面的键盘按钮：F13–F24 或任意快捷键），例如用于房间灯。不影响预设。
- 多个相同型号的设备可分别设置；设备按序列号识别。
- 断开连接的设备会连同其设置保存在归档中，重新连接后自动恢复。
- 无线设备休眠或关闭时显示“No link”（无连接）状态；设备唤醒后重新应用颜色。
- 睡眠（包括现代待机 Modern Standby）唤醒后以及插入设备时重新应用灯效。
- 更新：应用每天向 GitHub 查询一次最新版本；如有新版本，面板上会显示一张卡片，一键即可安装。可在托盘菜单中关闭此检查。
- 跟随显示器：Windows 调暗屏幕时灯光随之变暗，屏幕关闭时（空闲、锁定、睡眠）灯光关闭。可在托盘菜单中关闭此功能。
- 电量低于 10% 和 5% 时发出提醒。
- 托盘图标显示当前颜色。
- 固定模式：面板保持在最上层打开，可通过标题栏拖动。
- 将所有设置导出到文件及从文件导入。预设可以单独分享：右键单击预设 → Save to a file…（保存到文件），接收方通过托盘菜单中的 Load settings or a preset…（加载设置或预设）加载；预设会应用到接收方相同型号的设备上（如有多台，则全部应用），应用还会提示文件中有哪些型号是对方没有的。
- 英文和俄文界面：当 Windows 或其某个键盘布局为俄语时使用俄文，否则使用英文；可在托盘菜单中切换。
- 浅色和深色主题，跟随 Windows 设置。

## 安装

1. 从 [Releases](../../releases) 下载 `RGBTray-Setup-<version>.exe`。
2. 运行它。应用会为当前用户安装到 `%LOCALAPPDATA%\Programs\RGB Tray`，无需管理员权限，安装后自动启动。
3. 可在 Windows 设置 → 应用中卸载。

安装程序目前尚未进行代码签名，因此 Windows SmartScreen 可能会显示警告（更多信息 → 仍要运行）。设置保存在 `%APPDATA%\rgbtray` 中，更新和重新安装后都会保留。其中的 `backups` 文件夹保存最近 14 天以及每次更新前的设置副本；如果设置文件损坏，应用会自动使用最近一份可用的副本。

使用 RGB Tray 前请关闭厂商的灯效软件：两个程序同时写入同一设备会相互冲突。在 Windows 11 上，“Logitech LampArray”服务（动态光效）会导致 G915 忽略灯效；应用会对此发出警告并提供禁用选项。

## 命令行

| 命令 | 作用 |
|---|---|
| `rgbtray --diagnose [file.txt]` | 写入设备报告：应用和 Windows 版本、支持的设备，以及按接口分组的所有 HID 设备，最可能的灯效控制器排在最前。不指定文件名时，报告保存到 `%APPDATA%\rgbtray\rgbtray-diagnose.txt` 并在记事本中打开。也可从托盘菜单中使用。 |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | 发送原始 HID 报告并将应答追加到文件中，用于分析设备协议（[docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)）。 |
| `rgbtray --profile-check <out.txt> <profile.toml>` | 读取配置文件而不访问任何设备，并写出应用对其中每个型号的解析结果，或其中的错误。 |
| `rgbtray --profile-test <out.txt> <profile.toml>` | 在尚未安装的 `[hid]` 配置文件所对应的设备上播放颜色和灯效，并记录每一步。 |
| `rgbtray --show` | 启动时打开并固定面板。 |
| `rgbtray --uninstall` | 卸载应用（保留设置）。 |

## 从源码构建

需要 Windows 10/11 和 stable 版 [Rust](https://rustup.rs)，工具链为 `x86_64-pc-windows-gnu` 或 `-msvc`。安装 rustup 后，请打开一个新终端，使 `cargo` 位于 PATH 中。

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

安装程序就是同一个可执行文件：当文件名为 `RGBTray-Setup-*.exe`（或以 `--install` 参数启动）时，它会自行安装。

## 参与贡献

欢迎提交设备报告、在你的硬件上测试、报告问题以及提交 pull request。[CONTRIBUTING.md](CONTRIBUTING.md) 说明了如何构建和检查应用、如何添加设备，以及 pull request 需要满足的要求。

## 致谢

设备协议参考了以下项目；RGB Tray 未包含其中任何代码：

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) — Logitech、Razer 和 ASUS Aura 灯效
- [OpenRazer](https://github.com/openrazer/openrazer) — Razer 协议
- [Solaar](https://github.com/pwr-Solaar/Solaar) — Logitech HID++
- [QMK](https://qmk.fm) 和 [VIA](https://www.caniusevia.com) — MacroPad 灯效协议

## 许可证

本项目可依你的选择，按 [MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE) 任一许可证授权。

除非你另有明确声明，否则你有意提交以纳入本项目的任何贡献（按 Apache-2.0 许可证的定义）均应按上述方式双重许可，且不附加任何额外条款或条件。

本项目与 Logitech、Razer、ASUS、Geekboard 或 Tuya 均无关联。商标归其各自所有者所有。

本文为译文，如有出入，以英文版 README 为准。
