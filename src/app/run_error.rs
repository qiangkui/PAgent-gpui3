//! What kind of failure a workflow run hit.
//!
//! The engine already describes a step failure with a short `kind` (`credential`,
//! `assert`, `timeout`, …), but a run-level failure is only a sentence. The
//! window maps both to this small vocabulary so the banner and transcript can
//! say what the reader can act on, not just what threw.

/// The actionable categories a workflow failure is shown under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunErrorKind {
    /// A missing or invalid endpoint key, or a missing secret input.
    Credential,
    /// A required input is empty or malformed.
    Input,
    /// Node, Python, the engine directory, or another local dependency.
    Environment,
    /// The manifest, its schema, or its input/step declarations.
    Workflow,
    /// A request could not reach the endpoint.
    Network,
    /// The model answered badly, was unavailable, or refused the shape.
    Model,
    /// A request exceeded its deadline.
    Timeout,
    /// A captcha or login session needs a person.
    Login,
    /// A browser/page element was not found.
    Page,
    /// A program step exited non-zero or its command could not run.
    Execution,
    /// The workflow's own assertion or count check failed.
    Assert,
    /// The engine process or its bridge failed.
    Engine,
    /// The reader stopped the run.
    Cancelled,
    /// Nothing more specific is known.
    Unknown,
}

impl RunErrorKind {
    /// The label shown before the message.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Credential => "凭据/密钥问题",
            Self::Input => "输入问题",
            Self::Environment => "运行环境问题",
            Self::Workflow => "工作流/清单问题",
            Self::Network => "网络/端点问题",
            Self::Model => "模型/回答不合规",
            Self::Timeout => "超时",
            Self::Login => "登录/验证码问题",
            Self::Page => "页面/元素问题",
            Self::Execution => "步骤执行失败",
            Self::Assert => "校验未通过",
            Self::Engine => "引擎问题",
            Self::Cancelled => "已取消",
            Self::Unknown => "运行错误",
        }
    }

    /// One concrete next step, for the card when the engine gave no hint.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Credential => "到「设置 → AI」填好 Key 并测试，或确认工作流的 secret 输入。",
            Self::Input => "检查右侧输入是否完整、类型是否正确。",
            Self::Environment => "检查 Node、引擎目录 DSH_FLOW_HOME 和本机依赖。",
            Self::Workflow => "用 validate 检查 flow.yml、输入声明和步骤引用。",
            Self::Network => "检查代理、baseUrl、网络和端点状态。",
            Self::Model => "检查模型是否可用；必要时调整 prompt、schema 后重试。",
            Self::Timeout => "放宽 ai.timeoutMs 或步骤超时，并检查网络。",
            Self::Login => "先在对话里登录并复用 workspace；shell 运行不会带上界面凭据。",
            Self::Page => "检查选择器、页面结构和登录会话。",
            Self::Execution => "查看命令、stdout/stderr 和退出码，确认路径、权限和依赖。",
            Self::Assert => "看期望/实际和步骤输出，修正数据或筛选条件。",
            Self::Engine => "查看引擎日志；确认 cli.ts、Node 版本和 DSH_FLOW_HOME。",
            Self::Cancelled => "重新运行；已停止的步骤不会继续。",
            Self::Unknown => "展开日志和引擎错误，再按具体步骤排查。",
        }
    }

    /// Map an engine `error.kind` when it is specific enough.
    fn from_kind(kind: &str) -> Option<Self> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "credential" | "auth" | "api-key" | "api_key" => Some(Self::Credential),
            "input" | "missing-input" => Some(Self::Input),
            "environment" | "runtime" | "node" => Some(Self::Environment),
            "workflow" | "manifest" | "yaml" => Some(Self::Workflow),
            "network" | "http" | "connection" => Some(Self::Network),
            "model" | "response" | "schema" | "parse" => Some(Self::Model),
            "timeout" => Some(Self::Timeout),
            "login" | "captcha" | "session" => Some(Self::Login),
            "selector" | "selector-miss" | "page" | "browser" => Some(Self::Page),
            "run" | "exec" | "command" | "process" => Some(Self::Execution),
            "assert" | "verification" => Some(Self::Assert),
            "engine" => Some(Self::Engine),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            // `config` is used both by step scripts and by the loader; the
            // message is more specific, so let `from_message` decide.
            _ => None,
        }
    }

    /// Classify a sentence when the structured kind is absent or generic.
    pub(crate) fn from_message(message: &str) -> Self {
        let text = message.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
        if has(&[
            "no api key",
            "api key",
            "缺少 deepseek",
            "credential",
            "unauthorized",
            "401",
            "403",
            "密钥",
            "凭据",
        ]) {
            return Self::Credential;
        }
        if has(&[
            "dsh_engine_dir",
            "engine not found",
            "cannot start node",
            "node version",
            "找不到引擎",
            "运行环境",
            "python",
        ]) {
            return Self::Environment;
        }
        if has(&["timeout", "timed out", "超时", "etimedout"]) {
            return Self::Timeout;
        }
        if has(&["captcha", "验证码", "login", "登录", "未登录"]) {
            return Self::Login;
        }
        if has(&[
            "econnreset",
            "econnrefused",
            "network",
            "fetch failed",
            "网络",
            "端点",
        ]) {
            return Self::Network;
        }
        if has(&["selector", "元素", "页面", "browser", "chromium", "定位"]) {
            return Self::Page;
        }
        if has(&[
            "exit code",
            "exit status",
            "退出码",
            "进程退出",
            "stderr",
            "command failed",
        ]) {
            return Self::Execution;
        }
        if has(&["assert", "校验", "不一致", "期望", "expected", "得不到"]) {
            return Self::Assert;
        }
        if has(&[
            "schema",
            "模型",
            "model",
            "回答",
            "content_filter",
            "length",
        ]) {
            return Self::Model;
        }
        if has(&[
            "manifest",
            "flow.yml",
            "清单",
            "workflow",
            "工作流",
            "validate",
        ]) {
            return Self::Workflow;
        }
        if has(&["input", "输入", "必填"]) {
            return Self::Input;
        }
        if has(&["cancelled", "canceled", "已取消"]) {
            return Self::Cancelled;
        }
        Self::Unknown
    }

    /// Classify a structured step error, preferring its kind and falling back
    /// to the sentence.
    pub(crate) fn classify(kind: &str, message: &str) -> Self {
        if let Some(kind) = Self::from_kind(kind) {
            return kind;
        }
        Self::from_message(message)
    }

    /// Prefix a message with the category, for banners and run-level rows.
    pub(crate) fn prefixed(self, message: &str) -> String {
        let message = message.trim();
        if message.is_empty() {
            self.label().to_string()
        } else {
            format!("{}：{message}", self.label())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_structured_step_errors() {
        assert_eq!(RunErrorKind::classify("credential", ""), RunErrorKind::Credential);
        assert_eq!(RunErrorKind::classify("timeout", ""), RunErrorKind::Timeout);
        assert_eq!(RunErrorKind::classify("selector-miss", ""), RunErrorKind::Page);
        assert_eq!(RunErrorKind::classify("run", ""), RunErrorKind::Execution);
        assert_eq!(RunErrorKind::classify("assert", ""), RunErrorKind::Assert);
        assert_eq!(RunErrorKind::classify("engine", ""), RunErrorKind::Engine);
    }

    #[test]
    fn classifies_run_level_sentences() {
        assert_eq!(
            RunErrorKind::from_message("缺少 DeepSeek API Key：在界面里填写"),
            RunErrorKind::Credential
        );
        assert_eq!(
            RunErrorKind::from_message("找不到引擎：D:\\dsh-flow\\src\\cli.ts"),
            RunErrorKind::Environment
        );
        assert_eq!(
            RunErrorKind::from_message("读取验证码超时"),
            RunErrorKind::Timeout
        );
        assert_eq!(
            RunErrorKind::from_message("工作流清单 flow.yml 缺少 steps"),
            RunErrorKind::Workflow
        );
        assert_eq!(RunErrorKind::from_message("已取消"), RunErrorKind::Cancelled);
    }

    #[test]
    fn prefixes_only_when_there_is_a_message() {
        assert_eq!(RunErrorKind::Input.prefixed("缺少必填输入"), "输入问题：缺少必填输入");
        assert_eq!(RunErrorKind::Timeout.prefixed(""), "超时");
    }
}