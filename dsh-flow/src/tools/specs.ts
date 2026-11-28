/** The tool catalogue sent with every request. */

import type { ToolDefinition } from './types.ts'
import {
  HTTP_TIMEOUT_MS, LONG_RUN_TIMEOUT_MS, MAX_GREP_CONTEXT, MAX_HTTP_TIMEOUT_MS, MAX_RESULTS,
  MAX_SHELL_TIMEOUT_MS, MAX_TASK_LINES, MAX_TREE_DEPTH, MAX_TREE_ENTRIES, SHELL_TIMEOUT_MS
} from './limits.ts'

/** The tool catalogue sent with every request. */
export const TOOLS: ToolDefinition[] = [
  {
    type: 'function',
    function: {
      name: 'read',
      description:
        '读取工作目录里的一个文本文件，返回带行号的内容。PDF 也能读：自动按页抽取文字（需要本机装有 poppler 的 pdftotext 或 Python 的 pypdf），返回带「第 N 页」标记的内容，用 pages 指定范围；扫描件没有文字层时会说明改用 read_image 或附件。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的路径' },
          offset: { type: 'integer', description: '从第几行开始（1 起，可选；只对文本文件生效）' },
          limit: { type: 'integer', description: '最多读多少行（可选；只对文本文件生效）' },
          pages: {
            type: 'string',
            description: 'PDF 页码范围，例如 3 或 3-7（可选；只对 .pdf 生效，一次最多 40 页）'
          }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'write',
      description:
        '在工作目录里新建或覆盖一个文本文件（目录会自动创建）。写回一律 UTF-8；覆盖非 UTF-8 文件时会在结果里说明编码已变。',
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
        '改一个文件：用 old / new 替换文本（默认要求原文唯一，replaceAll 替换全部），用 edits 一次改多处，或用 startLine / endLine 替换指定行、insertAfter 在某行后插入。两种定位方式不能混在同一次调用里。文件不是 UTF-8（例如 GBK）时只支持 ASCII 原文（old / new / content 里不能有中文），按字节改且不动其余编码；要改中文先用 shell 转成 UTF-8。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的路径' },
          old: { type: 'string', description: '要被替换的原文，默认必须唯一（与 edits / startLine 二选一）' },
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
          },
          startLine: {
            type: 'integer',
            description: '按行编辑：从第几行开始（1 起，含）；与 old / new / edits 二选一'
          },
          endLine: {
            type: 'integer',
            description: '按行编辑：到第几行结束（含，可选，默认与 startLine 相同）'
          },
          insertAfter: {
            type: 'integer',
            description: '按行插入：在第几行之后插入（0 = 文件开头；与 startLine 二选一）'
          },
          content: {
            type: 'string',
            description: '按行编辑的替换 / 插入文本；要删除这几行时传空字符串'
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
        '在工作区的多个文件里批量替换同一段文本：pattern 用通配符选文件，path 可限定目录；默认每个文件里的原文必须唯一（多处时跳过并说明），replaceAll 为 true 时替换每个文件里的所有出现。二进制、超过 2 MB 的文件会跳过并说明。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '选文件的通配符，例如 src/**/*.ts' },
          path: {
            type: 'string',
            description: '只在这个目录下替换（可选，相对工作区）'
          },
          old: { type: 'string', description: '要被替换的原文' },
          new: { type: 'string', description: '替换后的文本' },
          replaceAll: {
            type: 'boolean',
            description: '替换每个文件里的所有出现（可选；默认每个文件必须唯一）'
          },
          dryRun: {
            type: 'boolean',
            description: '只返回会改哪些文件、各几处，不写盘（可选，不需要确认）'
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
      name: 'copy',
      description:
        '复制一个文件，字节原样复制（图片、字体等二进制资源也能复制）；每次执行前确认。目标已存在时拒绝，overwrite 为 true 才覆盖。',
      parameters: {
        type: 'object',
        properties: {
          from: { type: 'string', description: '源文件路径' },
          to: { type: 'string', description: '目标路径，父目录会自动创建' },
          overwrite: { type: 'boolean', description: '目标已存在时覆盖（可选，默认拒绝）' }
        },
        required: ['from', 'to']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'mkdir',
      description: '新建一个文件夹（父目录一并创建）；已存在时直接返回。每次执行前确认。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的文件夹路径' }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'delete',
      description:
        '删除文件；recursive 为 true 时删除整个文件夹（每次执行前确认；不能撤销）。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '要删除的文件或文件夹路径' },
          recursive: {
            type: 'boolean',
            description: '删除文件夹及其内容（可选，默认 false；只删单个文件）'
          }
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
      description: '按通配符列出工作目录里的文件，返回相对路径；path 可把搜索限定在一个目录。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '例如 **/*.ts 或 src/*.json' },
          path: {
            type: 'string',
            description: '只在这个目录下找（可选，相对工作区；不给就搜整个工作区）'
          }
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
      description:
        '按正则搜索工作目录里的文件内容，返回 路径:行号: 内容；path 可把搜索限定在一个目录。每个文件默认最多列 20 行（perFile 可调），避免一个文件吃掉全部结果。',
      parameters: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: '正则表达式' },
          path: {
            type: 'string',
            description: '只搜这个目录（可选，相对工作区；不给就搜整个工作区）'
          },
          glob: { type: 'string', description: '只搜索匹配该通配符的文件（可选）' },
          perFile: {
            type: 'integer',
            description: '每个文件最多列多少行（可选，默认 20，最多 200；上下文行也计入）'
          },
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
      name: 'git_write',
      description:
        '改动仓库的三种安全操作（每次执行前确认）：commit（按需 add 后提交）、branch（新建分支，checkout 为 true 时直接切过去）、stash（push / pop / list）。不做 reset、强制推送，也不会跳过 hooks。',
      parameters: {
        type: 'object',
        properties: {
          action: { type: 'string', description: 'commit、branch 或 stash' },
          message: { type: 'string', description: 'commit / stash push 的说明' },
          paths: {
            type: 'array',
            description: 'commit 前只 add 这些仓库内相对路径（可选）',
            items: { type: 'string' }
          },
          all: { type: 'boolean', description: 'commit 前 add -A（可选）' },
          name: { type: 'string', description: 'branch 的分支名' },
          checkout: { type: 'boolean', description: 'branch 后切到新分支（可选）' },
          stashAction: { type: 'string', description: 'stash 的动作：push（默认）/ pop / list' },
          includeUntracked: { type: 'boolean', description: 'stash push 时也收起未跟踪文件（可选）' },
          cwd: { type: 'string', description: '仓库里的运行目录（可选，默认当前目录）' }
        },
        required: ['action']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'shell',
      description:
        '在当前目录运行一条命令，返回退出码和输出；每次执行前都要用户确认。用来跑测试、构建或格式化。需要输入时用 stdin 一次给完（不是交互式终端）；background 为 true 时命令在后台继续运行（例如 dev server），它的 stdin 先由这里的 stdin 填入，之后用 task action=input 继续写，用 task 工具看输出或结束它。',
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
        '管理 shell 启动的后台任务：list 列出全部，output 看某个任务的输出尾部，input 往它的 stdin 写一行（只有读取 stdin 的程序收得到），close 关闭它的 stdin（让它读到 EOF 继续），wait 等它结束，stop 结束一个，stopAll 结束全部。input / close / stop 执行前确认。',
      parameters: {
        type: 'object',
        properties: {
          action: {
            type: 'string',
            description: 'list、output、input、close、wait、stop 或 stopAll'
          },
          id: { type: 'string', description: '任务 id，例如 task-1（除 list 与 stopAll 外都需要）' },
          text: { type: 'string', description: 'input 要写进 stdin 的文本（会自动补一个换行）' },
          lines: {
            type: 'integer',
            description: `output 最多显示多少行（可选，默认 60，最多 ${MAX_TASK_LINES}）`
          },
          timeoutMs: {
            type: 'integer',
            description: 'wait 最多等多少毫秒（可选，默认 30000，上限 120000）'
          }
        },
        required: ['action']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'wait_for',
      description:
        '等某个东西真的就绪再继续：target=port 等端口能连上，url 等页面应答（状态码 <500 即可），output 等后台任务的日志出现某段文本。默认最多等 30 秒（timeoutMs 上限 120 秒），就绪或超时都会说明原因。等后台任务结束请用 task 的 action=wait。每次调用前确认。',
      parameters: {
        type: 'object',
        properties: {
          target: { type: 'string', description: 'port、url 或 output' },
          host: { type: 'string', description: 'target=port 的主机（可选，默认 127.0.0.1）' },
          port: { type: 'integer', description: 'target=port 时的端口' },
          url: { type: 'string', description: 'target=url 时的完整地址' },
          id: {
            type: 'string',
            description: 'target=output 时的后台任务 id（例如 task-1）'
          },
          pattern: { type: 'string', description: 'target=output 时要等的文本（不区分大小写）' },
          timeoutMs: {
            type: 'integer',
            description: '最多等多少毫秒（可选，默认 30000，上限 120000）'
          },
          intervalMs: { type: 'integer', description: '两次检查之间的间隔（可选，默认 500）' }
        },
        required: ['target']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'diagnostics',
      description:
        '跑编译器 / 类型检查并把问题解析成清单：自动识别 cargo check --message-format=json、go vet、tsc --noEmit（优先项目自带的 tsc）、dotnet build、mvn compile、gradle classes（含 Kotlin 的 `e:` 行）或项目自带的 eslint（compact 输出），也可以用 command 直接给命令（命令里带 --message-format=json 会自动按 cargo 的 JSON 解析）。返回退出码、错误/警告计数、按「文件:行:列 代码」列出的前 30 条（severity / limit 可调）与输出末尾。每次执行前确认。',
      parameters: {
        type: 'object',
        properties: {
          command: { type: 'string', description: '不自动识别时直接给的命令（可选）' },
          format: {
            type: 'string',
            description: 'auto（默认）/ cargo（解析 cargo JSON）/ text（tsc、rustc 文本）'
          },
          cwd: { type: 'string', description: '相对工作目录的运行目录（可选，默认当前目录）' },
          severity: { type: 'string', description: 'all（默认）、error 或 warning' },
          limit: { type: 'integer', description: '每类最多列多少条（可选，默认 30，上限 100）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${LONG_RUN_TIMEOUT_MS}）`
          }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'run_tests',
      description:
        '跑这个项目的测试，并把失败用例解析成清单：自动识别 cargo test / go test / dotnet test / mvn test / gradle test / package.json 的 test 脚本 / pytest，也可以用 command 直接给一条命令。返回退出码、通过/失败统计、失败用例与原因（最多列 20 条）、以及输出末尾。每次执行前确认。',
      parameters: {
        type: 'object',
        properties: {
          command: { type: 'string', description: '不自动识别时直接给的命令（可选）' },
          filter: {
            type: 'string',
            description: '只跑匹配这个名字的用例（可选；cargo 作为过滤参数，pytest 用 -k，npm 作为 -- <filter> 透传给脚本）'
          },
          cwd: { type: 'string', description: '相对工作目录的运行目录（可选，默认当前目录）' },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${LONG_RUN_TIMEOUT_MS}）`
          }
        }
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
        '发一个 HTTP 请求（GET / POST / PUT / PATCH / DELETE / HEAD；只允许 http / https；跟随最多 5 次重定向，但不执行页面脚本），返回状态码、响应头和正文（HTML 会转成文本，最多 20000 字符）；给 saveTo 时把正文原样写到文件、不显示正文。每次请求前都要用户确认。需要认证时用 auth 指定一个环境变量名（值不会出现在调用里、对话里或卡片上）：Authorization 默认按 Bearer 发送，scheme=basic 会对变量值做 base64（变量里放 user:pass），指定别的 header（例如 X-Api-Key）则原样发送；重定向换主机或降级到 http 时不再带凭据。直接把 authorization 写进 headers 仍会被拒绝。',
      parameters: {
        type: 'object',
        properties: {
          url: { type: 'string', description: '完整 URL，必须带 http:// 或 https://' },
          method: {
            type: 'string',
            description: 'GET（默认）、POST、PUT、PATCH、DELETE 或 HEAD'
          },
          saveTo: {
            type: 'string',
            description: '把响应正文原样写到这个路径（可选，相对工作区；目录会自动创建，最多 20 MB）'
          },
          headers: {
            type: 'object',
            description: '请求头（可选，名字 → 值；最多 20 个，单值 ≤ 2000 字符）'
          },
          body: { type: 'string', description: 'POST 的正文（可选，≤ 2 万字符）' },
          auth: {
            type: 'object',
            description:
              '认证（可选）：用环境变量名代替密钥本身，值不会写进调用或对话。这个请求会一直要求用户确认，即使在完全权限下。',
            properties: {
              env: { type: 'string', description: '环境变量名，例如 GITHUB_TOKEN（必填）' },
              header: {
                type: 'string',
                description: '要发送的请求头（可选，默认 Authorization；也可以用 X-Api-Key）'
              },
              scheme: {
                type: 'string',
                description:
                  'bearer（默认，Authorization 用）、basic（把变量值 base64，变量里放 user:pass）或 raw（原样发送）'
              }
            },
            required: ['env']
          },
          timeoutMs: {
            type: 'integer',
            description: `超时毫秒数（可选，默认 ${HTTP_TIMEOUT_MS}，最多 ${MAX_HTTP_TIMEOUT_MS}）`
          }
        },
        required: ['url']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'env',
      description:
        '一次性探出这台机器和这个目录的环境：平台、node / python / git / cargo 等运行时及版本、常见项目文件（package.json、Cargo.toml、pyproject.toml、.venv 等）与 package.json 里的脚本名。只读，不需要确认，适合任务开始时先看一眼，省掉逐个试命令。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '要探的文件夹（可选，默认工作区）' }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'read_image',
      description:
        '读取一张图片（PNG / JPEG / GIF / WebP / BMP），把图片本身附给模型：检查截图、图标、界面稿时用，文本文件仍用 read。只读，不需要确认；超过 5 MB 或 20 MP（按面积，视觉模型的上限）会拒绝，凭据/密钥类文件一律不读。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '相对工作目录的图片路径' }
        },
        required: ['path']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'screenshot',
      description:
        '截取屏幕并把图片附给模型（Windows）：默认截前台窗口、长边缩到 1600px、存成 JPEG，避免把屏幕上无关的窗口一起发出去。region=all 才截整屏；x / y / width / height 四个都给时只截那块区域（相对所选区域，像素），用来对准某个按钮或表格；path 可指定保存位置，默认存到对话目录的 .dsh-flow/shots/。只读，不需要确认。',
      parameters: {
        type: 'object',
        properties: {
          path: { type: 'string', description: '保存到这个路径（可选，默认 .dsh-flow/shots/）' },
          region: { type: 'string', description: 'active（默认，前台窗口）或 all（整个桌面）' },
          maxEdge: { type: 'integer', description: '长边最多多少像素（可选，默认 1600，320–4000）' },
          format: { type: 'string', description: 'jpg（默认，体积小）或 png（清晰但大）' },
          quality: { type: 'integer', description: 'JPEG 质量（可选，默认 80，30–100）' },
          x: { type: 'integer', description: '裁切起点距所选区域左边的像素（可选；与 y / width / height 一起给）' },
          y: { type: 'integer', description: '裁切起点距所选区域顶边的像素（可选）' },
          width: { type: 'integer', description: '裁切宽度，像素（可选；四个都给才裁切）' },
          height: { type: 'integer', description: '裁切高度，像素（可选）' }
        }
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'show',
      description:
        '把结果文件展示在对话里：传文件路径，这一步下面会列出文件卡片，用户点一下就能打开、也能定位到所在文件夹。交付结果时用它——写完报告、导出表格、生成图片或压缩包后调用，不要让用户去工作区自己找。只列出已存在的文件，不读内容、不写文件、不需要确认；一次最多 12 个。',
      parameters: {
        type: 'object',
        properties: {
          paths: {
            type: 'array',
            items: { type: 'string' },
            description: '要展示的文件路径（相对工作目录或绝对路径），最多 12 个'
          },
          note: { type: 'string', description: '想在卡片上方写的一句话（可选）' }
        },
        required: ['paths']
      }
    }
  },
  {
    type: 'function',
    function: {
      name: 'browser',
      description:
        '无头浏览器，驱动本机已安装的 Edge / Chrome（不下载浏览器）：open / goto 打开 http(s) 页面，snapshot 列出可点的元素（[编号]），click / type / press / select 用真实输入事件操作，text 读正文，eval 执行 JavaScript，wait_for 等文字或元素出现，console / network 看日志与请求，screenshot 把画面附给模型（ref 或 selector 只截一个元素——验证码、某个按钮用这个），downloads 看下载到的文件，wait_download 等某个下载写完，close 关闭。一个对话共用一个浏览器与一个页面：进程活过单个提问，页面、滚动位置和登录状态留到下一问（闲置 30 分钟或 close 才收掉），接上旧页面时结果里会说明。**允许下载**：网页下载的文件落在对话目录的 .dsh-flow/downloads（结果里会给出路径），要不要展示给人由你用 show 决定；不读取用户自己的浏览器配置。',
      parameters: {
        type: 'object',
        properties: {
          action: {
            type: 'string',
            description:
              'open（打开页面）/ goto（跳转）/ snapshot（列出可点元素）/ text（正文）/ click / type / press / select / eval / wait_for / console / network / screenshot / downloads（列出下载的文件）/ close'
          },
          url: { type: 'string', description: 'open 与 goto 的 URL，必须 http:// 或 https://' },
          ref: { type: 'integer', description: 'snapshot 给出的编号（click / type / select / wait_for / screenshot 用）' },
          selector: { type: 'string', description: 'CSS 选择器（可选，代替 ref）' },
          text: { type: 'string', description: 'type 要输入的文字；wait_for 时是要等的文字' },
          submit: { type: 'boolean', description: 'type 之后按回车（可选）' },
          key: {
            type: 'string',
            description: 'press 的按键：enter / tab / escape / backspace / delete / space / arrowup / arrowdown / arrowleft / arrowright / home / end / pageup / pagedown'
          },
          values: { type: 'array', items: { type: 'string' }, description: 'select 要选中的选项值（可多个）' },
          expression: { type: 'string', description: 'eval 要执行的 JavaScript，返回值原样带回' },
          width: { type: 'integer', description: '视口宽度（可选，CSS 像素，320–2400）' },
          height: { type: 'integer', description: '视口高度（可选，CSS 像素，240–2000）' },
          waitMs: { type: 'integer', description: '导航后额外等待的毫秒数（可选，0–10000）' },
          timeoutMs: { type: 'integer', description: 'wait_for 的超时毫秒（可选，默认 10000）' },
          fullPage: { type: 'boolean', description: 'screenshot 截整页（可选，默认只截视口）' },
          format: { type: 'string', description: 'screenshot 的格式：jpg（默认，体积小）或 png' },
          quality: { type: 'integer', description: 'JPEG 质量（可选，默认 70，30–100）' },
          maxEdge: { type: 'integer', description: '截图长边最多多少像素（可选，默认 1400）' }
        },
        required: ['action']
      }
    }
  }
]
