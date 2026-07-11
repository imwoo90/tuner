//! # Telegram Reply Prompt Builder Tests
//!
//! This module contains tests for building reply prompts.

#[cfg(test)]
mod tests {
    use crate::telegram::reply::build_reply_prompt;
    use teloxide::types::Message;

    #[test]
    fn test_build_reply_prompt_no_reply() {
        let msg_json = r#"{
            "message_id": 1,
            "date": 123456,
            "chat": {"id": 123, "type": "private"},
            "text": "hello bot"
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let prompt = build_reply_prompt(&msg, "hello bot");
        assert_eq!(prompt, "hello bot");
    }

    #[test]
    fn test_build_reply_prompt_with_reply_text() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "text": "this is my reply",
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "deploy tomorrow"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let prompt = build_reply_prompt(&msg, "this is my reply");
        assert!(prompt.contains("The user is replying to this quoted message:"));
        assert!(prompt.contains("> deploy tomorrow"));
        assert!(prompt.contains("The user's message:\nthis is my reply"));
    }

    #[test]
    fn test_build_reply_prompt_with_reply_caption() {
        let msg_json = r#"{
            "message_id": 3,
            "date": 123458,
            "chat": {"id": 123, "type": "private"},
            "text": "nice photo",
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "photo of project deployment"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let prompt = build_reply_prompt(&msg, "nice photo");
        assert!(prompt.contains("The user is replying to this quoted message:"));
        assert!(prompt.contains("> photo of project deployment"));
        assert!(prompt.contains("The user's message:\nnice photo"));
    }
}
