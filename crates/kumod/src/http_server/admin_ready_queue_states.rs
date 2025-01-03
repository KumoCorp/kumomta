use crate::ready_queue::ReadyQueueManager;
use axum::extract::{Json, Query};
use kumo_api_types::{QueueState, ReadyQueueStateRequest, ReadyQueueStateResponse};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use std::collections::HashMap;

/// Retrieve information about the states that apply to a set of
/// ready queues, or all queues if no specific queues were named
/// in the request.
#[utoipa::path(
    get,
    tag="inspect",
    path="/api/admin/ready-q-states/v1",
    params(ReadyQueueStateRequest),
    responses(
        (status = 200, description = "Obtained state information", body=ReadyQueueStateResponse),
    ),
)]
pub async fn readyq_states(
    _: TrustedIpRequired,
    Query(request): Query<ReadyQueueStateRequest>,
) -> Result<Json<ReadyQueueStateResponse>, AppError> {
    let mut states_by_ready_queue = HashMap::new();
    let queues = ReadyQueueManager::all_queues();

    for queue in queues {
        if !request.queues.is_empty()
            && request
                .queues
                .iter()
                .find(|name| name.as_str() == queue.name())
                .is_none()
        {
            continue;
        }

        fn add_state(
            states_by_ready_queue: &mut HashMap<String, HashMap<String, QueueState>>,
            queue_name: &str,
            state_name: &str,
            state: QueueState,
        ) {
            let entry = states_by_ready_queue
                .entry(queue_name.to_string())
                .or_default();
            entry.insert(state_name.to_string(), state);
        }

        let states = queue.states.lock();
        if let Some(s) = &states.connection_limited {
            add_state(
                &mut states_by_ready_queue,
                queue.name(),
                "connection_limited",
                QueueState {
                    context: s.context.clone(),
                    since: s.since,
                },
            );
        }
        if let Some(s) = &states.connection_rate_throttled {
            add_state(
                &mut states_by_ready_queue,
                queue.name(),
                "connection_rate_throttled",
                QueueState {
                    context: s.context.clone(),
                    since: s.since,
                },
            );
        }
    }

    Ok(Json(ReadyQueueStateResponse {
        states_by_ready_queue,
    }))
}
