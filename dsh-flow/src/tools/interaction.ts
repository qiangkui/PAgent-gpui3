/**
 * The tools the loop runs against the window rather than against the disk.
 *
 * They live apart from `specs.ts` because they are not declared by the request's
 * allow-list: asking, writing todos, submitting a plan and remembering are
 * always offered, and a plugin may not shadow any of them (see `plugins.ts`).
 */

import type { ToolDefinition } from './types.ts'

/** The models the window can act on besides files and commands. */
export const INTERACTION_TOOLS: ToolDefinition[] = [
  {
    type: 'function',
    function: {
      name: 'ask_user_question',
      description:
        '向用户提一个带选项的问题并等待回答。需要用户在几个方案中选择、或确认一个关键前提时使用；不要在能自己判断时滥用。',
      parameters: {
        type: 'object',
        properties: {
          question: { type: 'string', description: '要问用户的问题' },
          options: {
            type: 'array',
            description: '2–6 个选项，按推荐顺序排列',
            items: { type: 'string' }
          },
          allowCustom: {
            type: 'boolean',
            description: '是否允许用户自己输入（可选）'
          }
        },
        required: ['question', 'options']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'todo_write',
      description:
        '写入或替换本次对话的待办列表。开始多步任务时列出计划，之后每次完成一步就更新状态；只保留当前列表，不要追加旧项。',
      parameters: {
        type: 'object',
        properties: {
          items: {
            type: 'array',
            description: '完整的新列表，按执行顺序',
            items: {
              type: 'object',
              properties: {
                text: { type: 'string', description: '要做的事' },
                status: {
                  type: 'string',
                  enum: ['pending', 'in_progress', 'completed'],
                  description: '待办 / 进行中 / 完成'
                }
              },
              required: ['text', 'status']
            }
          }
        },
        required: ['items']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'exit_plan_mode',
      description:
        '提交一份完整计划，等待用户批准后再执行。只有在计划模式下、并且计划已经写成以 # 标题开头的 markdown 时才能调用。',
      parameters: {
        type: 'object',
        properties: {
          plan: { type: 'string', description: '完整计划，markdown，以 # 标题开头' }
        },
        required: ['plan']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'remember',
      description:
        '写入一条长期记忆，以后每次提问都会带上：用户的偏好、项目的固定约定、反复踩到的坑。只写一句可复用的结论，不要写这次任务的临时状态；已经记过的事不要重复写。',
      parameters: {
        type: 'object',
        properties: {
          text: { type: 'string', description: '要记住的一句话，尽量具体、可复用' }
        },
        required: ['text']
      }
    }
  }
]

/** Names the interaction tools, so an allow-list cannot hide them. */
export const INTERACTION_NAMES = new Set(INTERACTION_TOOLS.map((tool) => tool.function.name))