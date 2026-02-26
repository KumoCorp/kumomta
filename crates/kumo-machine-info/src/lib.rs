use anyhow::Context;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Method;
use serde::Deserialize;
use serde_with::serde_as;
use std::sync::LazyLock;
use sysinfo::System;

static MAC: LazyLock<[u8; 6]> = LazyLock::new(get_mac_address_once);

/// Obtain the mac address of the first non-loopback interface on the system.
/// If there are no candidate interfaces, fall back to the `gethostid()` function,
/// which will attempt to load a host id from a file on the filesystem, or if that
/// fails, resolve the hostname of the node to its IPv4 address using a reverse DNS
/// lookup, and then derive some 32-bit number from that address through unspecified
/// means.
fn get_mac_address_once() -> [u8; 6] {
    match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.bytes(),
        _ => {
            // Fall back to gethostid, which is not great, but
            // likely better than just random numbers
            let host_id = unsafe { libc::gethostid() }.to_le_bytes();
            let mac: [u8; 6] = [
                host_id[0], host_id[1], host_id[2], host_id[3], host_id[4], host_id[5],
            ];
            mac
        }
    }
}

pub fn get_mac_address() -> &'static [u8; 6] {
    &*MAC
}

#[derive(Debug)]
pub struct MachineInfo {
    pub hostname: String,
    pub mac_address: String,
    pub machine_uid: Option<String>,
    pub node_id: Option<String>,
    pub num_cores: usize,
    pub kernel_version: Option<String>,
    pub platform: String,
    pub distribution: String,
    pub os_version: String,
    pub total_memory_bytes: u64,
    pub container_runtime: Option<String>,
    pub cpu_brand: String,
    pub cloud_provider: Option<CloudProvider>,
}

#[derive(Debug)]
pub enum CloudProvider {
    /// AWS
    Aws(aws::IdentityDocument),
    /// MS Azure
    Azure(azure::InstanceMetadata),
    /// Google Cloud Platform
    Gcp(gcp::InstanceMetadata),
}

impl CloudProvider {
    fn augment_fingerprint(&self, components: &mut Vec<String>) {
        match self {
            Self::Aws(id) => {
                components.push(format!("aws_instance_id={}", id.instance_id));
            }
            Self::Azure(instance) => {
                components.push(format!("azure_vm_id={}", instance.compute.vm_id));
            }
            Self::Gcp(instance) => {
                components.push(format!("gcp_id={}", instance.instance_id));
            }
        }
    }
}

impl MachineInfo {
    pub fn fingerprint(&self) -> String {
        let mut components = vec![];
        if let Some(provider) = &self.cloud_provider {
            provider.augment_fingerprint(&mut components);
        }
        if let Some(uid) = &self.machine_uid {
            components.push(format!("machine_uid={uid}"));
        }
        if let Some(id) = &self.node_id {
            components.push(format!("node_id={id}"));
        }
        if components.is_empty() {
            components.push(format!("mac={}", self.mac_address));
        }
        components.join(",")
    }

    pub fn new() -> Self {
        let hostname = gethostname::gethostname().to_string_lossy().to_string();
        let mac = get_mac_address();
        let mac_address = format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        let machine_uid = machine_uid::get().ok();

        let arch = System::cpu_arch();

        let mut system = System::new();
        system.refresh_memory();
        system.refresh_cpu_all();

        let mut cpu_info = vec![];
        for cpu in system.cpus() {
            let info = cpu.brand().to_string();
            if cpu_info.contains(&info) {
                continue;
            }
            cpu_info.push(info);
        }
        let cpu_brand = cpu_info.join(", ");

        Self {
            hostname,
            machine_uid,
            mac_address,
            node_id: None,
            num_cores: num_cpus::get(),
            platform: format!("{}/{arch}", std::env::consts::OS),
            distribution: System::distribution_id(),
            os_version: System::long_os_version()
                .unwrap_or_else(|| std::env::consts::OS.to_string()),
            total_memory_bytes: system.total_memory(),
            container_runtime: in_container::get_container_runtime().map(|r| r.to_string()),
            kernel_version: System::kernel_version(),
            cpu_brand,
            cloud_provider: None,
        }
    }

    /// Concurrently query for a known cloud providers
    pub async fn query_cloud_provider(&mut self) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        {
            let tx = tx.clone();
            tokio::task::spawn(async move {
                if let Ok(id) = aws::IdentityDocument::query().await {
                    tx.send(CloudProvider::Aws(id)).ok();
                }
            });
        }
        {
            let tx = tx.clone();
            tokio::task::spawn(async move {
                if let Ok(id) = azure::InstanceMetadata::query().await {
                    tx.send(CloudProvider::Azure(id)).ok();
                }
            });
        }
        {
            let tx = tx.clone();
            tokio::task::spawn(async move {
                if let Ok(id) = gcp::InstanceMetadata::query().await {
                    tx.send(CloudProvider::Gcp(id)).ok();
                }
            });
        }
        drop(tx);

        tokio::select! {
            biased;

            // Prefer to get a positive result
            provider = rx.recv() => {
                self.cloud_provider = provider;
            }

            // Overall timeout if things are taking too long
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(3)) => {}
        };
    }
}

pub mod azure {
    use super::*;
    use serde_json::Value;
    use std::collections::BTreeMap;

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct InstanceMetadata {
        pub compute: Compute,
        pub network: Network,
    }

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Compute {
        pub az_environment: String,
        pub vm_id: String,
        pub vm_size: String,
        pub location: String,
        #[serde(flatten)]
        pub unknown_: BTreeMap<String, Value>,
    }

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Network {
        pub interface: Vec<NetworkInterface>,
    }

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct NetworkInterface {
        pub ipv4: Ipv4Info,
        pub mac_address: String,
        #[serde(flatten)]
        pub unknown_: BTreeMap<String, Value>,
    }

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Ipv4Info {
        pub ip_address: Vec<IpAddressInfo>,
        pub subnet: Vec<SubnetInfo>,
    }

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct IpAddressInfo {
        pub private_ip_address: String,
        #[serde(default)]
        pub public_ip_address: String,
    }
    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct SubnetInfo {
        pub address: String,
        pub prefix: String,
    }

    impl InstanceMetadata {
        pub async fn query_via(base_url: &str) -> anyhow::Result<Self> {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap();

            let mut headers = HeaderMap::new();
            headers.insert("Metadata", HeaderValue::from_static("true"));

            let request = client
                .request(
                    Method::GET,
                    &format!("{base_url}/metadata/instance?api-version=2021-02-01"),
                )
                .headers(headers)
                .build()?;
            let response = client.execute(request).await?;

            let status = response.status();

            let body_text = response
                .text()
                .await
                .context("failed to read response body")?;
            if status.is_client_error() || status.is_server_error() {
                anyhow::bail!("failed to query identity: {status:?} {body_text}");
            }

            Ok(serde_json::from_str(&body_text)?)
        }

        pub async fn query() -> anyhow::Result<Self> {
            Self::query_via("http://169.254.169.254").await
        }
    }

    #[cfg(test)]
    #[tokio::test]
    async fn test_metadata() {
        use mockito::Server;

        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/metadata/instance?api-version=2021-02-01")
            .match_header("Metadata", "true")
            .with_status(200)
            .with_body(
                r#"{
    "compute": {
        "azEnvironment": "AZUREPUBLICCLOUD",
        "additionalCapabilities": {
            "hibernationEnabled": "true"
        },
        "hostGroup": {
          "id": "testHostGroupId"
        },
        "extendedLocation": {
            "type": "edgeZone",
            "name": "microsoftlosangeles"
        },
        "evictionPolicy": "",
        "isHostCompatibilityLayerVm": "true",
        "licenseType":  "",
        "location": "westus",
        "name": "examplevmname",
        "offer": "UbuntuServer",
        "osProfile": {
            "adminUsername": "admin",
            "computerName": "examplevmname",
            "disablePasswordAuthentication": "true"
        },
        "osType": "Linux",
        "placementGroupId": "f67c14ab-e92c-408c-ae2d-da15866ec79a",
        "plan": {
            "name": "planName",
            "product": "planProduct",
            "publisher": "planPublisher"
        },
        "platformFaultDomain": "36",
        "platformSubFaultDomain": "",
        "platformUpdateDomain": "42",
        "priority": "Regular",
        "publicKeys": [{
                "keyData": "ssh-rsa 0",
                "path": "/home/user/.ssh/authorized_keys0"
            },
            {
                "keyData": "ssh-rsa 1",
                "path": "/home/user/.ssh/authorized_keys1"
            }
        ],
        "publisher": "Canonical",
        "resourceGroupName": "macikgo-test-may-23",
        "resourceId": "/subscriptions/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx/resourceGroups/macikgo-test-may-23/providers/Microsoft.Compute/virtualMachines/examplevmname",
        "securityProfile": {
            "secureBootEnabled": "true",
            "virtualTpmEnabled": "false",
            "encryptionAtHost": "true",
            "securityType": "TrustedLaunch"
        },
        "sku": "18.04-LTS",
        "storageProfile": {
            "dataDisks": [{
                "bytesPerSecondThrottle": "979202048",
                "caching": "None",
                "createOption": "Empty",
                "diskCapacityBytes": "274877906944",
                "diskSizeGB": "1024",
                "image": {
                  "uri": ""
                },
                "isSharedDisk": "false",
                "isUltraDisk": "true",
                "lun": "0",
                "managedDisk": {
                  "id": "/subscriptions/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx/resourceGroups/macikgo-test-may-23/providers/Microsoft.Compute/disks/exampledatadiskname",
                  "storageAccountType": "StandardSSD_LRS"
                },
                "name": "exampledatadiskname",
                "opsPerSecondThrottle": "65280",
                "vhd": {
                  "uri": ""
                },
                "writeAcceleratorEnabled": "false"
            }],
            "imageReference": {
                "id": "",
                "offer": "UbuntuServer",
                "publisher": "Canonical",
                "sku": "16.04.0-LTS",
                "version": "latest",
                "communityGalleryImageId": "/CommunityGalleries/testgallery/Images/1804Gen2/Versions/latest",
                "sharedGalleryImageId": "/SharedGalleries/1P/Images/gen2/Versions/latest",
                "exactVersion": "1.1686127202.30113"
            },
            "osDisk": {
                "caching": "ReadWrite",
                "createOption": "FromImage",
                "diskSizeGB": "30",
                "diffDiskSettings": {
                    "option": "Local"
                },
                "encryptionSettings": {
                  "enabled": "false",
                  "diskEncryptionKey": {
                    "sourceVault": {
                      "id": "/subscriptions/test-source-guid/resourceGroups/testrg/providers/Microsoft.KeyVault/vaults/test-kv"
                    },
                    "secretUrl": "https://test-disk.vault.azure.net/secrets/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx"
                  },
                  "keyEncryptionKey": {
                    "sourceVault": {
                      "id": "/subscriptions/test-key-guid/resourceGroups/testrg/providers/Microsoft.KeyVault/vaults/test-kv"
                    },
                    "keyUrl": "https://test-key.vault.azure.net/secrets/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx"
                  }
                },
                "image": {
                    "uri": ""
                },
                "managedDisk": {
                    "id": "/subscriptions/xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx/resourceGroups/macikgo-test-may-23/providers/Microsoft.Compute/disks/exampleosdiskname",
                    "storageAccountType": "StandardSSD_LRS"
                },
                "name": "exampleosdiskname",
                "osType": "Linux",
                "vhd": {
                    "uri": ""
                },
                "writeAcceleratorEnabled": "false"
            },
            "resourceDisk": {
                "size": "4096"
            }
        },
        "subscriptionId": "xxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxx",
        "tags": "baz:bash;foo:bar",
        "version": "15.05.22",
        "virtualMachineScaleSet": {
            "id": "/subscriptions/xxxxxxxx-xxxxx-xxx-xxx-xxxx/resourceGroups/resource-group-name/providers/Microsoft.Compute/virtualMachineScaleSets/virtual-machine-scale-set-name"
        },
        "vmId": "02aab8a4-74ef-476e-8182-f6d2ba4166a6",
        "vmScaleSetName": "crpteste9vflji9",
        "vmSize": "Standard_A3",
        "zone": ""
    },
    "network": {
        "interface": [{
            "ipv4": {
               "ipAddress": [{
                    "privateIpAddress": "10.144.133.132",
                    "publicIpAddress": ""
                }],
                "subnet": [{
                    "address": "10.144.133.128",
                    "prefix": "26"
                }]
            },
            "ipv6": {
                "ipAddress": [
                 ]
            },
            "macAddress": "0011AAFFBB22"
        }]
    }
}"#,
            )
            .create_async()
            .await;

        let id = InstanceMetadata::query_via(&server.url()).await.unwrap();
        eprintln!("{id:#?}");
        assert_eq!(id.compute.vm_id, "02aab8a4-74ef-476e-8182-f6d2ba4166a6");
    }
}

pub mod aws {
    use super::*;

    #[serde_as]
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct IdentityDocument {
        #[serde(default)]
        #[serde_as(as = "serde_with::DefaultOnNull<_>")]
        pub devpay_product_codes: Vec<String>,
        #[serde(default)]
        #[serde_as(as = "serde_with::DefaultOnNull<_>")]
        pub marketplace_product_codes: Vec<String>,
        pub availability_zone: String,
        pub private_ip: String,
        pub version: String,
        pub instance_id: String,
        #[serde(default)]
        #[serde_as(as = "serde_with::DefaultOnNull<_>")]
        pub billing_products: Vec<String>,
        pub instance_type: String,
        pub account_id: String,
        pub image_id: String,
        pub pending_time: DateTime<Utc>,
        pub architecture: String,
        pub kernel_id: Option<String>,
        pub ramdisk_id: Option<String>,
        pub region: String,
    }

    impl IdentityDocument {
        pub async fn query_via(base_url: &str) -> anyhow::Result<Self> {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap();

            // For IMDSv2, attempt to obtain a token first.
            // In IMDSv1, this is not required, so we allow
            // for this to fail and proceed to the next step
            // without it

            let mut token = None;

            {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "X-aws-ec2-metadata-token-ttl-seconds",
                    HeaderValue::from_static("60"),
                );

                let request = client
                    .request(Method::PUT, &format!("{base_url}/latest/api/token"))
                    .headers(headers)
                    .build()?;

                // Note that for client.execute() to error it is likely a timeout
                // or a routing issue: if this happens, then we assume that IMDS
                // is not present, so we don't try with a second request that will
                // encounter the same issue, but take longer.
                // So we propagate that particular error out and stop
                // further progress.
                let response = client.execute(request).await?;

                if response.status().is_success() {
                    if let Ok(content) = response.text().await {
                        token.replace(content.trim().to_string());
                    }
                } else {
                    // Some kind of protocol error: perhaps they are running
                    // IMDSv1 rather than v2, so continue below without a token
                }
            }

            let mut headers = HeaderMap::new();
            if let Some(token) = token.as_deref() {
                headers.insert("X-aws-ec2-metadata-token", HeaderValue::from_str(token)?);
            }

            let request = client
                .request(
                    Method::GET,
                    &format!("{base_url}/latest/dynamic/instance-identity/document"),
                )
                .headers(headers)
                .build()?;
            let response = client.execute(request).await?;

            let status = response.status();

            let body_text = response
                .text()
                .await
                .context("failed to read response body")?;
            if status.is_client_error() || status.is_server_error() {
                anyhow::bail!("failed to query identity: {status:?} {body_text}");
            }

            Ok(serde_json::from_str(&body_text)?)
        }

        pub async fn query() -> anyhow::Result<Self> {
            Self::query_via("http://169.254.169.254").await
        }
    }

    #[cfg(test)]
    #[tokio::test]
    async fn test_aws_identity_v1() {
        use mockito::Server;

        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/latest/dynamic/instance-identity/document")
            .with_status(200)
            .with_body(
                r#"{
    "devpayProductCodes" : null,
    "marketplaceProductCodes" : [ "1abc2defghijklm3nopqrs4tu" ],
    "availabilityZone" : "us-west-2b",
    "privateIp" : "10.158.112.84",
    "version" : "2017-09-30",
    "instanceId" : "i-1234567890abcdef0",
    "billingProducts" : null,
    "instanceType" : "t2.micro",
    "accountId" : "123456789012",
    "imageId" : "ami-5fb8c835",
    "pendingTime" : "2016-11-19T16:32:11Z",
    "architecture" : "x86_64",
    "kernelId" : null,
    "ramdiskId" : null,
    "region" : "us-west-2"
}"#,
            )
            .create_async()
            .await;

        let id = IdentityDocument::query_via(&server.url()).await.unwrap();
        eprintln!("{id:#?}");
        assert_eq!(id.instance_id, "i-1234567890abcdef0");
    }

    #[cfg(test)]
    #[tokio::test]
    async fn test_aws_identity_v2() {
        use mockito::Server;

        let token = "fake-token";
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("PUT", "/latest/api/token")
            .match_header("X-aws-ec2-metadata-token-ttl-seconds", "60")
            .with_status(200)
            .with_body(token)
            .create_async()
            .await;
        let _mock = server
            .mock("GET", "/latest/dynamic/instance-identity/document")
            .with_status(200)
            .match_header("X-aws-ec2-metadata-token", token)
            .with_body(
                r#"{
    "devpayProductCodes" : null,
    "marketplaceProductCodes" : [ "1abc2defghijklm3nopqrs4tu" ],
    "availabilityZone" : "us-west-2b",
    "privateIp" : "10.158.112.84",
    "version" : "2017-09-30",
    "instanceId" : "i-1234567890abcdef0",
    "billingProducts" : null,
    "instanceType" : "t2.micro",
    "accountId" : "123456789012",
    "imageId" : "ami-5fb8c835",
    "pendingTime" : "2016-11-19T16:32:11Z",
    "architecture" : "x86_64",
    "kernelId" : null,
    "ramdiskId" : null,
    "region" : "us-west-2"
}"#,
            )
            .create_async()
            .await;

        let id = IdentityDocument::query_via(&server.url()).await.unwrap();
        eprintln!("{id:#?}");
        assert_eq!(id.instance_id, "i-1234567890abcdef0");
    }
}

pub mod gcp {
    use super::*;

    #[derive(Debug)]
    pub struct InstanceMetadata {
        pub instance_id: String,
    }

    impl InstanceMetadata {
        pub async fn query_via(base_url: &str) -> anyhow::Result<Self> {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap();

            let mut headers = HeaderMap::new();
            headers.insert("Metadata-Flavor", HeaderValue::from_static("Google"));

            let request = client
                .request(
                    Method::GET,
                    &format!("{base_url}/computeMetadata/v1/instance/id"),
                )
                .headers(headers)
                .build()?;
            let response = client.execute(request).await?;
            let status = response.status();

            let instance_id = response
                .text()
                .await
                .context("failed to read response body")?
                .trim()
                .to_string();
            if status.is_client_error() || status.is_server_error() {
                anyhow::bail!("failed to query identity: {status:?} {instance_id}");
            }

            Ok(Self { instance_id })
        }

        pub async fn query() -> anyhow::Result<Self> {
            Self::query_via("http://metadata.google.internal").await
        }
    }

    #[cfg(test)]
    #[tokio::test]
    async fn test_gcp() {
        use mockito::Server;

        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/computeMetadata/v1/instance/id")
            .with_status(200)
            .with_body("some_id")
            .create_async()
            .await;

        let id = InstanceMetadata::query_via(&server.url()).await.unwrap();
        eprintln!("{id:#?}");
        assert_eq!(id.instance_id, "some_id");
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_machine_info() {
        use super::*;
        let info = MachineInfo::new();
        eprintln!("{}", info.fingerprint());
        eprintln!("{info:#?}");
        /* It's hard to make a test assertion that will run anywhere
         * because this code is all about being machine specific.
         * This is here to help me see what the output looks like
         * while hacking on this.
         */
        // panic!("{info:#?}");
    }
}
