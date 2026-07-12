use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct TaskCreateRequest {
    #[serde(rename = "from")]
    pub parent_agent: String,
    pub prompt: String,
    pub name: Option<String>,
    pub chat_id: Option<i64>,
    pub thread_id: Option<i64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub priority: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskCreateResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskResumeRequest {
    pub task_id: String,
    pub prompt: String,
    #[serde(rename = "from")]
    pub parent_agent: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskAskParentRequest {
    pub task_id: String,
    pub question: String,
}

#[derive(Debug, Serialize)]
pub struct TaskAskParentResponse {
    pub success: bool,
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskCancelRequest {
    pub task_id: String,
    #[serde(rename = "from")]
    pub parent_agent: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskDeleteRequest {
    pub task_id: String,
    #[serde(rename = "from")]
    pub parent_agent: String,
}

#[derive(Debug, Serialize)]
pub struct StandardResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub from: Option<String>,
}

impl TaskCreateRequest {
    pub fn into_submit(self) -> crate::tasks::TaskSubmit {
        crate::tasks::TaskSubmit {
            chat_id: self.chat_id.unwrap_or(0),
            prompt: self.prompt,
            message_id: 0,
            thread_id: self.thread_id,
            parent_agent: self.parent_agent,
            name: self.name.unwrap_or_default(),
            provider_override: self.provider.unwrap_or_default(),
            model_override: self.model.unwrap_or_default(),
            thinking_override: self.thinking.unwrap_or_default(),
            priority: self.priority.unwrap_or_default(),
            depends_on: self.depends_on,
        }
    }
}
