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

    use crate::telegram::reply::{strip_mention, prepend_reply_to_media};

    #[test]
    fn test_strip_mention_removes_tag() {
        assert_eq!(strip_mention("@mybot hello", Some("mybot")), "hello");
        assert_eq!(strip_mention("hello @MyBot", Some("mybot")), "hello");
        assert_eq!(strip_mention("/start@my_bot", Some("my_bot")), "/start");
        assert_eq!(strip_mention("/start@my_bot hello", Some("my_bot")), "/start hello");
    }

    #[test]
    fn test_strip_mention_no_tag() {
        assert_eq!(strip_mention("hello world", Some("mybot")), "hello world");
    }

    #[test]
    fn test_strip_mention_none_username() {
        assert_eq!(strip_mention("@mybot hello", None), "@mybot hello");
    }

    #[test]
    fn test_prepend_reply_to_media_voice() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "from": {"id": 1, "is_bot": false, "first_name": "T"},
            "voice": {"file_id": "v1", "file_unique_id": "vu1", "duration": 3, "mime_type": null},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Point 3: deploy Friday"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]\n...");
        assert!(out.contains("The user is replying to this quoted message:\n> Point 3: deploy Friday"));
        assert!(out.contains("Their reply is a voice message (the attached file below)."));
        assert!(out.ends_with("[INCOMING FILE]\n..."));
    }

    #[test]
    fn test_prepend_reply_to_media_photo() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "photo": [{"file_id": "p1", "file_unique_id": "pu1", "width": 100, "height": 100}],
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some photo context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is an image (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_document() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "document": {"file_id": "d1", "file_unique_id": "du1"},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some text context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is a document (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_audio() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "from": {"id": 1, "is_bot": false, "first_name": "T"},
            "audio": {"file_id": "a1", "file_unique_id": "au1", "duration": 10, "mime_type": null},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some text context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is an audio file (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_video() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "from": {"id": 1, "is_bot": false, "first_name": "T"},
            "video": {"file_id": "v1", "file_unique_id": "vu1", "width": 100, "height": 100, "duration": 10, "mime_type": null},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some text context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is a video (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_video_note() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "video_note": {"file_id": "vn1", "file_unique_id": "vnu1", "length": 100, "duration": 10},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some text context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is a video note (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_sticker() {
        let msg_json = r#"{
            "message_id": 2,
            "date": 123457,
            "chat": {"id": 123, "type": "private"},
            "from": {"id": 1, "is_bot": false, "first_name": "T"},
            "sticker": {"file_id": "s1", "file_unique_id": "su1", "width": 100, "height": 100, "is_animated": false, "is_video": false, "type": "regular"},
            "reply_to_message": {
                "message_id": 1,
                "date": 123456,
                "chat": {"id": 123, "type": "private"},
                "text": "Some text context"
            }
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        let out = prepend_reply_to_media(&msg, "[INCOMING FILE]");
        assert!(out.contains("Their reply is a sticker (the attached file below)."));
    }

    #[test]
    fn test_prepend_reply_to_media_no_reply() {
        let msg_json = r#"{
            "message_id": 1,
            "date": 123456,
            "chat": {"id": 123, "type": "private"},
            "photo": []
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        assert_eq!(prepend_reply_to_media(&msg, "BODY"), "BODY");
    }
}

