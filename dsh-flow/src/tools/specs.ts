/** The tool catalogue sent with every request. */

import type { ToolDefinition } from './types.ts'
import {
  HTTP_TIMEOUT_MS, MAX_GREP_CONTEXT, MAX_HTTP_TIMEOUT_MS, MAX_RESULTS, MAX_SHELL_TIMEOUT_MS,
  MAX_TASK_LINES, MAX_TREE_DEPTH, MAX_TREE_ENTRIES, SHELL_TIMEOUT_MS
} from './limits.ts'

/** The tool catalogue sent with every request. */
export const TOOLS: ToolDefinition[] = [
  {
    type: 'function',
    function: {
      name: 'read',
      description: '读取工作目录里的一个文本文件，返回带行号的内容。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的路径' },
          offset: { type: 'integer', description: '从第几行开始（1 起，可选）' },
          limit: { type: 'integer', description: '最多读多少行（可选）' }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'write',
      description: '在工作目录里新建或覆盖一个文本文件（目录会自动创建）。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的路径' },
          content: { type: 'string', description: '完整文件内容' }
        },
        required: ['path', 'content']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'edit',
      description:
        '替换文件里的文本：默认要求原文唯一；replaceAll 为 true 时替换全部出现。可以给 old / new 改一处，也可以用 edits 一次改多处。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的路径' },
          old: { type: 'string', description: '要被替换的原文，默认必须唯一（与 edits 二选一）' },
          new: { type: 'string', description: '替换后的文本' },
          replaceAll: {
            type: 'boolean',
            description: '替换所有出现（可选；edits 里每一项也可以单独写）'
          },
          edits: {
            type: 'array',
            description: '一次替换多处；默认每项都必须唯一，按顺序生效',
            items: {
              type: 'object',
              properties: {
                old: { type: 'string', description: '要被替换的原文' },
                new: { type: 'string', description: '替换后的文本' },
                replaceAll: { type: 'boolean', description: '替换这一项的所有出现（可选）' }
              },
              required: ['old', 'new']
            }
          }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'replace',
      description:
        '在工作区的多个文件里批量替换同一段文本：pattern 用通配符选文件；默认每个文件里的原文必须唯一（多处时跳过并说明），replaceAll 为 true 时替换每个文件里的所有出现。二进制、超过 2 MB 的文件会跳过并说明。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '选文件的通配符，例如 src/**/*.ts' },
          old: { type: 'string', description: '要被替换的原文' },
          new: { type: 'string', description: '替换后的文本' },
          replaceAll: {
            type: 'boolean',
            description: '替换每个文件里的所有出现（可选；默认每个文件必须唯一）'
          }
        },
        required: ['pattern', 'old', 'new']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'move',
      description: '移动或重命名一个文件（每次执行前确认；目标已存在时拒绝）。',
      parameters: {
        type: 'object',
        properties: {
          from: { type: 'string', description: '源文件路径' },
          to: { type: 'string', description: '目标路径，父目录会自动创建' }
        },
        required: ['from', 'to']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'delete',
      description: '删除一个文件（每次执行前确认；不能删除文件夹，删除后不能撤销）。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '要删除的文件路径' }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'list',
      description: '列出工作目录里某个文件夹的直接子项，不递归；目录以 / 结尾。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的文件夹路径，默认 .' }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'glob',
      description: '按通配符列出工作目录里的文件，返回相对路径。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '例如 **/*.ts 或 src/*.json' }
        },
        required: ['pattern']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'tree',
      description:
        '按树形列出工作目录（或指定文件夹）的结构，跳过 .git / node_modules / target，并遵守 .gitignore。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的文件夹，默认 .' },
          depth: {
            type: 'integer',
            description: `列到第几层（可选，默认 2，最多 ${MAX_TREE_DEPTH}）`
          },
          limit: {
            type: 'integer',
            description: `最多列多少项（可选，默认 ${MAX_TREE_ENTRIES}）`
          }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'grep',
      description: '按正则搜索工作目录里的文件内容，返回 路径:行号: 内容。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '正则表达式' },
          glob: { type: 'string', description: '只搜索匹配该通配符的文件（可选）' },
          ignoreCase: { type: 'boolean', description: '忽略大小写（可选）' },
          context: {
            type: 'integer',
            description: `每个命中前后各显示几行（可选，0–${MAX_GREP_CONTEXT}）`
          }
        },
        required: ['pattern']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'symbols',
      description:
        '按名字查找仓库里的声明（Rust / TS / JS / Python / Go 的函数、类、结构体、接口等）。首次调用会遍历工作区建立索引，缓存在 .dsh-flow/index.json，10 分钟内复用；refresh: true 强制重建。比 grep 更适合找“这个函数在哪定义”。',
      parameters: {
        type: 'object',
        properties: {
          query: { type: 'string', description: '符号名的一部分，不区分大小写（可选）' },
          find: {
            type: 'string',
            description:
              '传 references 时改成查这个标识符的声明与整词引用（需要 query 是完整标识符），适合改名前确认影响面'
          },
          path: { type: 'string', description: '只查这个目录下的文件（可选，相对工作区）' },
          refresh: { type: 'boolean', description: '重建索引（可选）' },
          limit: {
            type: 'integer',
            description: `最多返回多少条（可选，默认 ${MAX_RESULTS}）`
          }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'git',
      description:
        '只读地查看仓库：status / diff / log / show / ls-files / rev-parse / shortlog / describe / blame，不会改动工作区。',
      parameters: {
        type: 'object',
        properties: {
          command: { type: 'string', description: '子命令，例如 status、diff、log' },
          args: {
            type: 'array',
            description: '附加参数，例如 ["--stat"]（可选）',
            items: { type: 'string' }
          },
          path: { type: 'string', description: '只查看这个路径（可选）' },
          cwd: { type: 'string', description: '仓库里的运行目录（可选，默认当前目录）' }
        },
        required: ['command']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'shell',
      description:
        '在当前目录运行一条命令，返回退出码和输出；每次执行前都要用户确认。用来跑测试、构建或格式化。需要输入时用 stdin 一次给完（不是交互式终端）；background 为 true 时命令在后台继续运行（例如 dev server），之后用 task 工具看输出或结束它。',
      parameters: {
        type: 'object',
        properties: {
          command: { type: 'string', description: '要执行的命令行，例如 npm test' },
          cwd: { type: 'string', description: '相对工作目录的运行目录（可选，默认当前目录）' },
          stdin: {
            type: 'string',
            description: '一次性写进命令 stdin 的文本（可选）；写完即关闭，程序读到文件末尾就继续'
          },
          background: {
            type: 'boolean',
            description: '让命令在后台运行并在这次问答之后继续（可选；配合 task 工具使用）'
          },
          name: { type: 'string', description: '后台任务的名字，显示在任务行里（可选）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${SHELL_TIMEOUT_MS}，最多 ${MAX_SHELL_TIMEOUT_MS}；后台任务不受它限制）`
          }
        },
        required: ['command']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'task',
      description:
        '管理 shell 启动的后台任务：list 列出全部，output 看某个任务的输出尾部，stop 结束一个，stopAll 结束全部。',
      parameters: {
        type: 'object',
        properties: {
          action: { type: 'string', description: 'list、output、stop 或 stopAll' },
          id: { type: 'string', description: '任务 id，例如 task-1（output / stop 需要）' },
          lines: {
            type: 'integer',
            description: `output 最多显示多少行（可选，默认 60，最多 ${MAX_TASK_LINES}）`
          }
        },
        required: ['action']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'python',
      description:
        '在当前目录运行一段 Python 代码（python -c），返回退出码和输出；每次执行前都要用户确认。写小脚本验证想法时比 shell 少一层引号转义。',
      parameters: {
        type: 'object',
        properties: {
          code: { type: 'string', description: '要执行的 Python 源码，支持多行' },
          cwd: { type: 'string', description: '相对工作目录的运行目录（可选，默认当前目录）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${SHELL_TIMEOUT_MS}，最多 ${MAX_SHELL_TIMEOUT_MS}）`
          }
        },
        required: ['code']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'node',
      description:
        '在当前目录运行一段 Node.js 代码（node -e，按 CommonJS 执行，可用 require），返回退出码和输出；每次执行前都要用户确认。适合快速验证 JS 片段或调用本地脚本。',
      parameters: {
        type: 'object',
        properties: {
          code: { type: 'string', description: '要执行的 JavaScript 源码，支持多行' },
          cwd: { type: 'string', description: '相对工作目录的运行目录（可选，默认当前目录）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${SHELL_TIMEOUT_MS}，最多 ${MAX_SHELL_TIMEOUT_MS}）`
          }
        },
        required: ['code']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'http',
      description:
        '发一个 GET 或有限的 POST 请求（只允许 http / https，不跟随重定向，不发 authorization / cookie 这类可能带密钥的请求头），返回状态码、响应头和正文前 8000 字符；每次请求前都要用户确认。适合查文档、看一个页面的正文、调本地 dev server 的接口。',
      parameters: {
        type: 'object',
        properties: {
          url: { type: 'string', description: '完整 URL，必须带 http:// 或 https://' },
          method: { type: 'string', description: 'GET（默认）或 POST' },
          headers: {
            type: 'object',
            description: '请求头（可选，名字 → 值；最多 20 个，单值 ≤ 2000 字符）'
          },
          body: { type: 'string', description: 'POST 的正文（可选，≤ 2 万字符）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${HTTP_TIMEOUT_MS}，最多 ${MAX_HTTP_TIMEOUT_MS}）`
          }
        },
        required: ['url']
      }
    }
  }
]
