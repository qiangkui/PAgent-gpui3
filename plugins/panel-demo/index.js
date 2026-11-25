// 示例插件：在「上下文」页加一块只读面板。
//
// 面板每次提问构建一次（第一轮模型请求之前），只回报数字和短文本；窗口用
// 自己的样式画成数字摘要 + 条形 / 表格 / 文本三种 section。它不改变请求，
// 也不调用工具——那是 registerTool / injectPrompt 的职责。
export function activate(api) {
  api.registerPanel({ id: 'request_shape', title: '请求构成' }, (context) => {
    const turns = context.turns.slice(-8)
    return {
      summary: [
        {
          label: '窗口',
          value: context.contextWindow > 0 ? String(context.contextWindow) : '未知'
        },
        { label: '系统', value: String(context.systemChars) },
        { label: '工具定义', value: String(context.toolChars) }
      ],
      sections: [
        {
          title: '字符占比',
          kind: 'bars',
          items: [
            { label: '系统提示', value: context.systemChars, note: 'chars' },
            { label: '插件注入', value: context.injectedChars, note: 'chars' },
            { label: '工具定义', value: context.toolChars, note: 'chars' }
          ]
        },
        {
          title: '最近几轮',
          kind: 'table',
          columns: ['角色', '字符'],
          rows:
            turns.length > 0
              ? turns.map((turn) => [turn.role, String(turn.chars)])
              : [['（还没有历史）', '0']]
        },
        {
          title: '说明',
          kind: 'text',
          items: [
            {
              label: '这个面板做什么',
              text: '只读展示：按本问的请求构成画一张图，不改提示词，也不调用工具。'
            }
          ]
        }
      ]
    }
  })
}