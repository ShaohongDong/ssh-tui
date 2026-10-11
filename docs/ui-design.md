# 紧凑 UI 与统一主题

本轮设计参考 OpenAI Codex `806d973` 的 [文字层级](https://github.com/openai/codex/blob/806d9732c974bc8a51b8317c1bd8985544fe627c/codex-rs/tui/styles.md)、[列表布局](https://github.com/openai/codex/blob/806d9732c974bc8a51b8317c1bd8985544fe627c/codex-rs/tui/src/bottom_pane/list_selection_view.rs)、[响应式页脚](https://github.com/openai/codex/blob/806d9732c974bc8a51b8317c1bd8985544fe627c/codex-rs/tui/src/bottom_pane/footer.rs)、[主题预览](https://github.com/openai/codex/blob/806d9732c974bc8a51b8317c1bd8985544fe627c/codex-rs/tui/src/theme_picker.rs) 和 [对比度处理](https://github.com/openai/codex/blob/806d9732c974bc8a51b8317c1bd8985544fe627c/codex-rs/tui/src/style/contrast.rs)。组件在本项目的 Ratatui 结构内独立实现。

## 布局与交互

| 界面 | 行为 |
| --- | --- |
| 终端 | 一行标签、一行状态；活动标签保持可见，溢出显示左右标记 |
| 连接列表 | 内容自适应高度；80×24 中两条连接占八行；最多十条可见，长列表滚动 |
| 列表列宽 | 按显示宽度对齐名称、目标和路由；窄窗口逐级隐藏次要列 |
| 表单 | 64 列以上同行编辑，窄窗口上下排列；滚动字段并保持输入光标可见 |
| 主题 | 96 列以上并排预览，窄窗口上下排列；上下或 j/k 循环预览，PageUp/PageDown 翻页，Home/End 首尾跳转；取消恢复，确认保存 |
| 确认与帮助 | 确认按内容收缩；帮助、详细信息支持行、页和首尾滚动 |
| 状态 | 保存提示三秒后消失；错误可通过 F2 或 Ctrl+] ! 展开，下一次操作清除 |

窗口不足 40 列或主区域不足八行时，弹层取消边框和外边距，将空间交给内容和操作提示。24×6 下列表显示一个焦点条目，表单显示一个字段，仍保留 Enter/Esc 操作。更小的窗口保持渲染稳定，无法保证全部文字可见。

## 主题与预览

十二种内置主题及旧自定义主题可用。新增 Catppuccin Mocha、One Dark、Monokai、Kanagawa（Wave）、Everforest Dark（Medium）、Rose Pine；原有六种主题顺序及 Tokyo Night 默认值保持兼容。配色来源见 [README](../README.md#color-themes)。可选 `[ui]` 提供面板、边框、选择前景/背景及三种语义状态色；其余 UI 颜色使用原主题派生。文字颜色根据实际绘制背景校正，远程 VT100 屏幕与调色板保持原规则。

主题标题显示当前位置和包含自定义主题的总数。窄窗口先保留位置提示，再缩短主题名；极窄窗口仅显示位置。翻页与渲染共用布局计算，按实际可见列表行数移动，至少一步，到边界停止。即时预览不修改配置、不重连会话；Enter 保存失败时保持选择器和预览，Esc 仍可恢复进入时的主题。

以下图片由真实 Ratatui `TestBackend` 的 80×24 缓冲区生成，字体与实际外层终端可能不同。

![Tokyo Night 连接列表](ui-dark.png)

![Solarized Light 连接列表](ui-light.png)

## 检查方式

正常 `cargo test --locked` 对照 `tests/snapshots/ui-*.txt`，覆盖 80×24、120×36、48×12、24×6 的空列表、两条连接、大量长名称、代理、长输入、确认、帮助首尾和主题首尾预览，两种语言及语言选择器共 88 个视图。另检查真实单元格的前景/背景对比度、活动标签和所有字段光标，以及极小窗口渲染。主题交互测试覆盖翻页与窗口尺寸、长自定义主题列表、位置提示、保存重启和写入失败恢复。

有意调整布局时使用 `UPDATE_UI_SNAPSHOTS=1 cargo test --locked ui_snapshots_cover_responsive_views` 更新快照，并人工审查差异。该命令也将深色和浅色缓冲区导出至 `target/ui-{dark,light}.tsv`，用于视觉核验。

实际 PTY 交互回归在 [验收记录](validation.md) 中记录，包括 80×24 紧凑尺寸、48×12 验证错误详情、帮助滚动和主题取消。

## 中英文界面

界面默认英文，支持 `language = "en"` / `"zh"` 和临时 `--language` 覆盖。连接与代理列表使用 `l`、远程终端使用 `Ctrl+] l` 打开语言选择器。上下键预览，Enter 保存，Esc 恢复，返回进入前的页面；切换不会重连现有 SSH 会话。英文提示采用短标签，24×6 中保留 Enter/Esc 操作及输入光标。

快照按 `tests/snapshots/ui-{en,zh}-{width}x{height}.txt` 分开保存，共覆盖 88 个视图。上方深色和浅色预览展示默认英文界面。界面生成的状态与错误保留文案标识，显示时翻译；用户数据、远端输出和既有终端内容保持原样。代理子进程使用连接建立时传入的语言。
