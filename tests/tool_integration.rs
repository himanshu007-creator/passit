use std::sync::Arc;

use passit::db::database::Database;
use passit::db::messages::{add_message, get_messages_by_session, NewMessage};
use passit::db::sessions::{create_session, get_session, list_sessions, CreateSessionParams, SessionFilter};
use passit::tools;

fn get_text_content(content: &[rmcp::model::Content]) -> String {
    content.first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

async fn setup_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().unwrap())
}


#[tokio::test]
async fn test_save_and_load_tool_integration() {
    let db = setup_db().await;

    // Save a user turn (creates a new session)
    let result = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: None,
            role: "user".into(),
            content: "Hello, can you help me with Rust?".into(),
            agent_id: Some("test-agent".into()),
            model: Some("claude-3.5".into()),
            tokens_in: Some(50),
            tokens_out: None,
            project_path: Some("/project".into()),
            tags: Some(vec!["rust".into()]),
        },
        "default-agent",
    )
    .await
    .unwrap();

    let save_result: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let session_id = save_result["session_id"].as_str().unwrap().to_string();
    assert!(save_result["is_new_session"].as_bool().unwrap());
    assert_eq!(save_result["turn_index"], 0);

    // Save an assistant response
    let result = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: Some(session_id.clone()),
            role: "assistant".into(),
            content: "Sure! I can help you with Rust. What specifically?".into(),
            agent_id: Some("test-agent".into()),
            model: Some("claude-3.5".into()),
            tokens_in: None,
            tokens_out: Some(200),
            project_path: None,
            tags: None,
        },
        "default-agent",
    )
    .await
    .unwrap();

    let save_result: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert!(!save_result["is_new_session"].as_bool().unwrap());
    assert_eq!(save_result["turn_index"], 1);

    // Load the session
    let load_result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session_id.clone(),
            format: Some("messages".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let load_json: serde_json::Value =
        serde_json::from_str(&get_text_content(&load_result.content)).unwrap();
    assert_eq!(load_json["session"]["id"], session_id);
    assert_eq!(load_json["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_list_tool() {
    let db = setup_db().await;

    // No sessions yet
    let result = tools::list::list_sessions_tool(
        &db,
        tools::list::ListSessionsParams {
            limit: None,
            offset: None,
            project_path: None,
            agent: None,
            tag: None,
            since: None,
            source: None,
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["total"], 0);

    // Create a session
    create_session(
        &db,
        CreateSessionParams {
            title: "Test".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    let result = tools::list::list_sessions_tool(
        &db,
        tools::list::ListSessionsParams {
            limit: None,
            offset: None,
            project_path: None,
            agent: None,
            tag: None,
            since: None,
            source: None,
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["total"], 1);
}

#[tokio::test]
async fn test_branch_tool() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Original".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    // Add some messages
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "First".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Response".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    // Branch from turn 0 (copies both messages)
    let result = tools::branch::branch_session(
        &db,
        tools::branch::BranchSessionParams {
            session_id: session.id.clone(),
            from_turn: 0,
            branch_title: Some("Forked".into()),
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["messages_copied"], 2);
    assert_eq!(json["branch_title"], "Forked");

    // Branch from turn 1 (copies only assistant message)
    let result = tools::branch::branch_session(
        &db,
        tools::branch::BranchSessionParams {
            session_id: session.id.clone(),
            from_turn: 1,
            branch_title: None,
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["messages_copied"], 1);
}

#[tokio::test]
async fn test_search_tool() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Search Test".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "How do I implement retry logic with exponential backoff?".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id,
            role: "assistant".into(),
            content: "Here's a retry implementation with exponential backoff".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::search::search_sessions(
        &db,
        tools::search::SearchSessionsParams {
            query: "retry".into(),
            limit: Some(10),
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["results"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_export_tool() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Export Test".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec!["test".into()],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Hello".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::export_tool::export_session(
        &db,
        tools::export_tool::ExportSessionParams {
            session_id: session.id.clone(),
            format: Some("json".into()),
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["format"], "json");
    assert!(json["content"].as_str().unwrap().contains("Export Test"));

    let md_result = tools::export_tool::export_session(
        &db,
        tools::export_tool::ExportSessionParams {
            session_id: session.id,
            format: Some("markdown".into()),
        },
    )
    .await
    .unwrap();
    let md_json: serde_json::Value =
        serde_json::from_str(&get_text_content(&md_result.content)).unwrap();
    assert_eq!(md_json["format"], "markdown");
}

#[tokio::test]
async fn test_import_tool() {
    let db = setup_db().await;

    let export_json = serde_json::json!({
        "version": "1.0",
        "session": {
            "id": "ses_imported",
            "title": "Imported Session",
            "agent_origin": "other-agent",
            "project_path": "/other",
            "tags": ["imported"],
        },
        "messages": [
            {
                "role": "user",
                "content": "Hello from import",
                "agent_id": "other",
            },
            {
                "role": "assistant",
                "content": "Imported response",
                "agent_id": "other",
            },
        ],
    });

    let result = tools::import_tool::import_session(
        &db,
        tools::import_tool::ImportSessionParams {
            content: export_json.to_string(),
            merge: Some(false),
        },
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["imported_messages"], 2);
    assert!(!json["was_merge"].as_bool().unwrap());

    // Verify the data persisted
    let session = get_session(&db, "ses_imported").unwrap().unwrap();
    assert_eq!(session.title, "Imported Session");
    let msgs = get_messages_by_session(&db, "ses_imported").unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].content, "Hello from import");
}

#[tokio::test]
async fn test_full_handoff_cycle() {
    let db = setup_db().await;

    // Agent A saves a conversation
    let result_a = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: None,
            role: "user".into(),
            content: "What's the best way to handle errors in Rust?".into(),
            agent_id: Some("claude-code".into()),
            model: Some("claude-3.5".into()),
            tokens_in: Some(30),
            tokens_out: None,
            project_path: Some("/rust-project".into()),
            tags: Some(vec!["rust".into(), "error-handling".into()]),
        },
        "claude-code",
    )
    .await
    .unwrap();
    let saved_a: serde_json::Value =
        serde_json::from_str(&get_text_content(&result_a.content)).unwrap();
    let sid = saved_a["session_id"].as_str().unwrap().to_string();
    assert!(saved_a["is_new_session"].as_bool().unwrap());

    tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: Some(sid.clone()),
            role: "assistant".into(),
            content: "Use Result<T, E> and the `?` operator for most cases.".into(),
            agent_id: Some("claude-code".into()),
            model: Some("claude-3.5".into()),
            tokens_in: None,
            tokens_out: Some(150),
            project_path: None,
            tags: None,
        },
        "claude-code",
    )
    .await
    .unwrap();

    // Agent B (opencode) loads the session with explicit handoff format
    let loaded = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: Some("handoff".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();
    let loaded_json: serde_json::Value =
        serde_json::from_str(&get_text_content(&loaded.content)).unwrap();
    assert!(loaded_json["transcript"].as_str().unwrap().contains("handle errors"));
    assert!(loaded_json["instruction"].as_str().unwrap().contains("HANDOFF"));

    // Agent B continues the conversation
    let result_b = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: Some(sid.clone()),
            role: "user".into(),
            content: "What about custom error types?".into(),
            agent_id: Some("opencode".into()),
            model: Some("gemini-2.5".into()),
            tokens_in: Some(20),
            tokens_out: None,
            project_path: None,
            tags: None,
        },
        "claude-code",
    )
    .await
    .unwrap();
    let saved_b: serde_json::Value =
        serde_json::from_str(&get_text_content(&result_b.content)).unwrap();
    assert!(!saved_b["is_new_session"].as_bool().unwrap());
    assert_eq!(saved_b["turn_index"], 2);

    // Verify all 3 messages in the session
    let msgs = get_messages_by_session(&db, &sid).unwrap();
    assert_eq!(msgs.len(), 3);

    // Verify session filtering works per-agent
    let (claude_sessions, _) = list_sessions(
        &db,
        SessionFilter {
            agent: Some("claude-code".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(claude_sessions.len(), 1);
}

#[tokio::test]
async fn test_save_session_not_found_error() {
    let db = setup_db().await;

    let result = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: Some("ses_nonexistent".into()),
            role: "user".into(),
            content: "test".into(),
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            project_path: None,
            tags: None,
        },
        "agent",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_load_claude_session() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Rust Refactoring".into(),
            agent_origin: "claude-code".into(),
            project_path: Some("/Users/user/.claude/projects/passit".into()),
            tags: vec!["rust".into()],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Refactor this module to use builder pattern.".into(),
            content_type: None,
            agent_id: Some("claude-code".into()),
            model: Some("claude-sonnet-4".into()),
            tokens_in: Some(40),
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Here's the refactored module with Builder pattern.".into(),
            content_type: None,
            agent_id: Some("claude-code".into()),
            model: Some("claude-sonnet-4".into()),
            tokens_in: None,
            tokens_out: Some(500),
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id.clone(),
            format: Some("handoff".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("claude-code"), "origin label");
    assert!(transcript.contains("Rust Refactoring"), "title");
    assert!(instruction.contains("HANDOFF"), "handoff instruction");
    assert!(transcript.contains("builder pattern"), "session content");
}

#[tokio::test]
async fn test_load_gemini_session() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "API Design Review".into(),
            agent_origin: "gemini-cli".into(),
            project_path: Some("/Users/user/.gemini/projects/api".into()),
            tags: vec!["api".into(), "design".into()],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Review the REST API design for our new service.".into(),
            content_type: None,
            agent_id: Some("gemini-cli".into()),
            model: Some("gemini-2.5-pro".into()),
            tokens_in: Some(60),
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "The API design looks solid. A few suggestions:\n1. Use /v1/ prefix\n2. Add pagination to list endpoints\n3. Consider rate limiting".into(),
            content_type: None,
            agent_id: Some("gemini-cli".into()),
            model: Some("gemini-2.5-pro".into()),
            tokens_in: None,
            tokens_out: Some(400),
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id.clone(),
            format: Some("briefing".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("══ BRIEFING ══"), "briefing header");
    assert!(transcript.contains("API Design Review"), "title");
    assert!(transcript.contains("gemini-cli"), "origin label");
    assert!(instruction.contains("LATEST"), "briefing instruction points to latest message");
}

#[tokio::test]
async fn test_load_opencode_session() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Database Migration".into(),
            agent_origin: "opencode".into(),
            project_path: None,
            tags: vec!["db".into()],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Write a migration for adding users table.".into(),
            content_type: None,
            agent_id: Some("opencode".into()),
            model: Some("gpt-4o".into()),
            tokens_in: Some(30),
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "CREATE TABLE users (id SERIAL PRIMARY KEY, email TEXT UNIQUE);".into(),
            content_type: None,
            agent_id: Some("opencode".into()),
            model: Some("gpt-4o".into()),
            tokens_in: None,
            tokens_out: Some(200),
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id.clone(),
            format: Some("compact".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("HANDOFF"), "compact header");
    assert!(instruction.contains("CONTINUE"), "continue instruction");
    // Compact format keeps goal label minimal
    assert!(transcript.contains("Database Migration"), "title");
}

// Format-specific tests

#[tokio::test]
async fn test_load_briefing_format() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Architecture Review".into(),
            agent_origin: "agent".into(),
            project_path: Some("/project".into()),
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Review the microservices architecture for latency.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Key findings: 1) Add caching layer 2) Use connection pooling.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id.clone(),
            format: Some("briefing".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("══ BRIEFING ══"), "briefing header with double equals");
    assert!(transcript.contains("Architecture Review"), "title");
    assert!(transcript.contains("microservices"), "session content");
    assert!(instruction.contains("LATEST"), "briefing instruction points to latest message");
}

#[tokio::test]
async fn test_load_compact_format() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Quick Fix".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Fix the null pointer in the parser.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Added null check before dereferencing the pointer.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id,
            format: Some("compact".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("HANDOFF"));
    assert!(transcript.contains("Quick Fix"));
    assert!(instruction.contains("CONTINUE"));
    assert!(!transcript.contains("━━━"), "compact has no box drawing");
}

#[tokio::test]
async fn test_load_transcript_format() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "Debug Session".into(),
            agent_origin: "agent".into(),
            project_path: None,
            tags: vec![],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "The app crashes on startup.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Check the config file format.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "It's missing a required field.".into(),
            content_type: None,
            agent_id: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id,
            format: Some("transcript".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    let instruction = json["instruction"].as_str().unwrap();
    assert!(transcript.contains("=== Session:"), "transcript header");
    assert!(transcript.contains("[Turn 0 | user]"), "turn 0 label");
    assert!(transcript.contains("[Turn 1 | assistant]"), "turn 1 label");
    assert!(transcript.contains("[Turn 2 | user]"), "turn 2 label");
    assert!(transcript.contains("The app crashes"), "turn 1 content");
    assert!(transcript.contains("Check the config"), "turn 2 content");
    assert!(transcript.contains("missing a required field"), "turn 3 content");
    assert!(instruction.contains("transcript"), "transcript instruction");
}

#[tokio::test]
async fn test_load_messages_format() {
    let db = setup_db().await;

    let session = create_session(
        &db,
        CreateSessionParams {
            title: "JSON Test".into(),
            agent_origin: "test-agent".into(),
            project_path: Some("/test".into()),
            tags: vec!["json".into()],
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        },
    )
    .unwrap();

    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "user".into(),
            content: "Test message one.".into(),
            content_type: None,
            agent_id: Some("agent-a".into()),
            model: Some("model-x".into()),
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();
    add_message(
        &db,
        NewMessage {
            session_id: session.id.clone(),
            role: "assistant".into(),
            content: "Test message two.".into(),
            content_type: None,
            agent_id: Some("agent-a".into()),
            model: Some("model-x".into()),
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        },
    )
    .unwrap();

    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: session.id,
            format: Some("messages".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert_eq!(json["session"]["title"], "JSON Test");
    assert_eq!(json["session"]["agent_origin"], "test-agent");
    assert_eq!(json["session"]["tags"], serde_json::json!(["json"]));
    let messages = json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "Test message one.");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], "Test message two.");
}

#[tokio::test]
async fn test_elicit_protocol() {
    let db = setup_db().await;

    // Save a couple of turns
    let result = tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: None,
            role: "user".into(),
            content: "Let's implement a binary search in Rust.".into(),
            agent_id: Some("agent-a".into()),
            model: Some("claude".into()),
            tokens_in: Some(10),
            tokens_out: None,
            project_path: Some("/rust".into()),
            tags: None,
        },
        "agent-a",
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let sid = json["session_id"].as_str().unwrap().to_string();

    tools::save::save_session_turn(
        &db,
        tools::save::SaveSessionTurnParams {
            session_id: Some(sid.clone()),
            role: "assistant".into(),
            content: "Here's an implementation:\n\n```rust\nfn binary_search<T: Ord>(arr: &[T], target: &T) -> Option<usize> {\n    let mut low = 0;\n    let mut high = arr.len();\n    while low < high {\n        let mid = low + (high - low) / 2;\n        match arr[mid].cmp(target) {\n            std::cmp::Ordering::Less => low = mid + 1,\n            std::cmp::Ordering::Greater => high = mid,\n            std::cmp::Ordering::Equal => return Some(mid),\n        }\n    }\n    None\n}\n```".into(),
            agent_id: Some("agent-a".into()),
            model: Some("claude".into()),
            tokens_in: None,
            tokens_out: Some(300),
            project_path: None,
            tags: None,
        },
        "agent-a",
    )
    .await
    .unwrap();

    // Without a peer (test/CLI context), format=None defaults to handoff
    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: None,
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        true,
        None,
    )
    .await
    .unwrap();
    let text = get_text_content(&result.content);
    assert!(text.contains("HANDOFF"), "should contain handoff content");
    assert!(text.contains("binary search"), "should contain session content");

    // With explicit format, content is returned directly
    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: Some("compact".into()),
            max_content_length: None,
            from_turn: None,
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert!(json["transcript"].as_str().unwrap().contains("binary search"));
    assert_eq!(json["instruction"].as_str().unwrap(), "[CONTINUE] Respond to last turn directly. No re-analysis, no greeting.");

    // from_turn=0 loads both turns
    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: Some("compact".into()),
            max_content_length: None,
            from_turn: Some(0),
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    assert!(json["transcript"].as_str().unwrap().contains("binary search"));

    // from_turn=1 loads only the assistant turn
    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: Some("transcript".into()),
            max_content_length: None,
            from_turn: Some(1),
        },
        "test-agent",
        2000,
        1000,
        300,
        false,
        None,
    )
    .await
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&get_text_content(&result.content)).unwrap();
    let transcript = json["transcript"].as_str().unwrap();
    // Title is auto-generated from first user message so "Let's implement" appears in the session header
    assert!(transcript.contains("Let's implement"), "title from session");
    // Only 1 message in transcript (turn 1, the assistant turn)
    assert!(transcript.contains("binary_search"), "assistant turn included");
    assert_eq!(transcript.matches("[Turn").count(), 1, "only one turn");
    assert!(!transcript.contains("[Turn 0 | user]"), "user turn excluded");

    // format=None + from_turn=Some(1): defaults to handoff, loads from turn 1
    let result = tools::load::load_session(
        &db,
        tools::load::LoadSessionParams {
            session_id: sid.clone(),
            format: None,
            max_content_length: None,
            from_turn: Some(1),
        },
        "test-agent",
        2000,
        1000,
        300,
        true,
        None,
    )
    .await
    .unwrap();
    let text = get_text_content(&result.content);
    assert!(text.contains("HANDOFF"), "should contain handoff content");
    assert!(text.contains("binary search"), "should include session goal");
}
