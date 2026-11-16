//! 回放查不存在的工作项答 404 `board_task_not_found`，不答 503 `board_unavailable`
//! （BUG-037）：控制库读得出、只是没有这一项，是确定的结论，不是「稍后重试」。
use super::TestBoard;
use serde_json::json;

/// 格式合法、从未建过的工作项号。
const UNKNOWN_TASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

#[tokio::test]
async fn replay_of_an_unknown_task_is_not_found_rather_than_unavailable() {
    let f = TestBoard::new().await;
    let login = f.writer.http.issue().await;
    let uri = format!("/api/v1/board/replay?task_id={UNKNOWN_TASK}");
    let body = f.writer.get(&uri, &login.cookie, 404).await;
    assert_eq!(
        body,
        json!({"code": "board_task_not_found", "error": "工作项不存在。"})
    );
    // 同一平面上真实存在的工作项照常回放：上面的 404 只来自「没有这一项」。
    let create = json!({"request_id": "replay-not-found", "spec": f.spec()});
    let task = f.call("create", create, 200).await;
    let task_id = task.as_str().unwrap();
    let uri = format!("/api/v1/board/replay?task_id={task_id}");
    let replay = f.writer.get(&uri, &login.cookie, 200).await;
    assert_eq!(replay["task"]["task"]["task_id"], task);
    f.stop().await;
}
