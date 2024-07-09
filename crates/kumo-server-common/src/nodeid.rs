use once_cell::sync::Lazy;
use std::path::PathBuf;
use uuid::Uuid;

static NODEID: Lazy<NodeId> = Lazy::new(|| NodeId::new());
const DEFAULT_NODE_ID_PATH: &str = "/opt/kumomta/etc/.nodeid";

/// The NodeId is intended to identify a specific instance of KumoMTA
/// within your own local cluster.
/// It is a uuid that will be generated and persisted when the node
/// starts up.
///
/// If persisting the id isn't possible, we fall back to generating
/// a "stable" v1 uuid from the mac address or deriving a fake mac address
/// from the hostid of the system. Those aren't great when running in
/// some virtualization environments, so it is recommended to resolve
/// any issues with persisting the id there. There are some environment
/// variables that can be used to influence that if the default filesystem
/// path is not suitable for whatever reason.
///
/// The intended use of the nodeid is disambiguation during reporting,
/// and also for future configuration management/provisioning related
/// functionality.
#[derive(Debug, Clone)]
pub struct NodeId {
    /// Unique node id in the cluster
    pub uuid: Uuid,

    /// Captures any write error we may have experienced while generating
    /// the uuid. This is surfaced by the `check` method.
    write_error: Option<String>,
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.uuid.fmt(fmt)
    }
}

impl NodeId {
    /// Get the NodeId
    pub fn get() -> Self {
        (*NODEID).clone()
    }

    /// Retrieve just the uuid
    pub fn get_uuid() -> Uuid {
        NODEID.uuid
    }

    /// Raises an error if we don't have a persistent unique node id
    pub fn check() -> anyhow::Result<()> {
        let nodeid = Self::get();
        if let Some(err) = &nodeid.write_error {
            anyhow::bail!(
                "Unable to determine the KUMO_NODE_ID. \
                 Refusing to operate as part of a cluster. {err}"
            );
        }
        Ok(())
    }

    pub fn new() -> Self {
        let mut write_error = None;

        let uuid = match std::env::var_os("KUMO_NODE_ID") {
            Some(id_os) => match id_os.to_str() {
                Some(id) => match Uuid::parse_str(id) {
                    Ok(uuid) => uuid,
                    Err(err) => {
                        panic!("Env var KUMO_NODE_ID (`{id}`) is not a valid UUID: {err:#}")
                    }
                },
                None => panic!("Env var KUMO_NODE_ID (`{id_os:?}`) is not valid UTF-8"),
            },
            None => {
                let uuid_path: PathBuf = match std::env::var_os("KUMO_NODE_ID_PATH") {
                    Some(node_path) => node_path.into(),
                    None => DEFAULT_NODE_ID_PATH.into(),
                };

                match std::fs::read_to_string(&uuid_path) {
                    Ok(id) => match Uuid::parse_str(&id) {
                        Ok(uuid) => uuid,
                        Err(err) => {
                            panic!("File {uuid_path:?} content `{id}` is not a valid UUID: {err:#}")
                        }
                    },
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        let uuid = Uuid::new_v4();

                        match std::fs::write(&uuid_path, format!("{uuid}")) {
                            Ok(_) => uuid,
                            Err(err)
                                if err.kind() == std::io::ErrorKind::PermissionDenied
                                    || err.kind() == std::io::ErrorKind::NotFound =>
                            {
                                let err =
                                    format!("Failed to write node id to {uuid_path:?}: {err:#}");
                                tracing::debug!(
                                    "{err}. Proceeding on the assumption that \
                                     we're not in a cluster and switching to a \
                                     stable v1 uuid based on the mac address"
                                );
                                write_error.replace(err);

                                // Switch to a mac address based v1 uuid, with a fixed
                                // timestamp. It looks like:
                                // 00000000-0000-1000-8000-XXXXXXXXXXXX
                                // where the X's are the hex digits from the mac address
                                uuid_helper::new_v1(uuid::Timestamp::from_rfc4122(0, 0))
                            }
                            Err(err) => panic!("Failed to write node id to {uuid_path:?}: {err:#}"),
                        }
                    }
                    Err(err) => panic!("File {uuid_path:?} could not be read: {err:#}"),
                }
            }
        };

        Self { uuid, write_error }
    }
}
