# Skills（源码）

这里的每个文件夹是一个可导入的 Skill：`skill.json` 提供名称与描述，`SKILL.md` 提供注入到提示词里的指令。

在窗口的「Skill」页点标题旁的 **+**，选中其中一个文件夹，会复制到 `<程序目录>/skills/` 并在列表里出现；打开开关后才会进入每次提问。

| Skill | 作用 |
|---|---|
| `workflow-distill/` | 把聊天里跑通的流程固化成语义最小的 dsh-flow 清单（先 `dsh-flow distill`，再按判据 review） |