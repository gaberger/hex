#![allow(clippy::too_many_arguments)]
use spacetimedb::{table, reducer, ReducerContext, Table};

#[table(name = test_session, public)]
#[derive(Clone, Debug)]
pub struct TestSession {
    #[unique]
    pub id: String,
    pub agent_id: String,
    pub commit_hash: String,
    pub branch: String,
    pub started_at: String,
    pub finished_at: String,
    pub trigger: String,
    pub overall_status: String,
    pub pass_count: u32,
    pub fail_count: u32,
    pub skip_count: u32,
    pub total_count: u32,
    pub duration_ms: u64,
}

#[table(name = test_result, public)]
#[derive(Clone, Debug)]
pub struct TestResult {
    #[unique]
    pub id: String,
    pub session_id: String,
    pub category: String,
    pub name: String,
    pub status: String,
    pub duration_ms: u64,
    pub error_message: String,
    pub file_path: String,
}

#[reducer]
pub fn record_session(
    ctx: &ReducerContext,
    id: String,
    agent_id: String,
    commit_hash: String,
    branch: String,
    started_at: String,
    finished_at: String,
    trigger: String,
    overall_status: String,
    pass_count: u32,
    fail_count: u32,
    skip_count: u32,
    total_count: u32,
    duration_ms: u64,
) -> Result<(), String> {
    ctx.db.test_session().insert(TestSession {
        id,
        agent_id,
        commit_hash,
        branch,
        started_at,
        finished_at,
        trigger,
        overall_status,
        pass_count,
        fail_count,
        skip_count,
        total_count,
        duration_ms,
    });
    Ok(())
}

#[reducer]
pub fn record_result(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    category: String,
    name: String,
    status: String,
    duration_ms: u64,
    error_message: String,
    file_path: String,
) -> Result<(), String> {
    ctx.db.test_result().insert(TestResult {
        id,
        session_id,
        category,
        name,
        status,
        duration_ms,
        error_message,
        file_path,
    });
    Ok(())
}

#[reducer]
pub fn prune_old_sessions(ctx: &ReducerContext, keep_count: u32) -> Result<(), String> {
    let mut sessions: Vec<TestSession> = ctx.db.test_session().iter().collect();
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if sessions.len() <= keep_count as usize {
        return Ok(());
    }

    let to_remove = &sessions[keep_count as usize..];
    for session in to_remove {
        // Delete associated test results first
        let results: Vec<TestResult> = ctx
            .db
            .test_result()
            .iter()
            .filter(|r| r.session_id == session.id)
            .collect();
        for result in results {
            ctx.db.test_result().id().delete(&result.id);
        }
        // Delete the session
        ctx.db.test_session().id().delete(&session.id);
    }

    Ok(())
}
