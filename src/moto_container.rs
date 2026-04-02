use aws_config::{Region, SdkConfig};
use aws_credential_types::Credentials;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, ReuseDirective};
use tokio::sync::OnceCell;

static AWS_CONFIG: LazyLock<Mutex<HashMap<String, &'static OnceCell<SdkConfig>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const MOTO_PORT: u16 = 5000;

/// Start a Moto mock server (if not already running) and return an `SdkConfig`
/// pointing at it.
///
/// The container is started once per `container_name` and reused for all
/// subsequent calls with the same name. The returned config uses static
/// `"testing"` credentials and the `us-east-1` region — you may override
/// these with your own `aws_config` builder if needed.
///
/// # Example
///
/// ```rust,ignore
/// use test_containers_util::moto_container::get_aws_config;
/// use aws_sdk_s3::Client as S3Client;
///
/// #[tokio::test]
/// async fn my_aws_test() {
///     let config = get_aws_config("my-moto").await;
///     let s3 = S3Client::new(&config);
///     s3.create_bucket().bucket("my-bucket").send().await.unwrap();
/// }
/// ```
pub async fn get_aws_config(container_name: &str) -> SdkConfig {
    let cell = {
        let mut map = AWS_CONFIG.lock().unwrap();
        *map.entry(container_name.to_string())
            .or_insert_with(|| Box::leak(Box::new(OnceCell::new())))
    };

    cell.get_or_init(|| async {
        let container = get_container(container_name).await;
        let host_port = container
            .get_host_port_ipv4(MOTO_PORT)
            .await
            .expect("Moto should be listening");
        let endpoint_url = format!("http://127.0.0.1:{}", host_port);
        create_config(endpoint_url.as_str()).await
    })
    .await
    .clone()
}

/// Start (or re-attach to) the Moto container with the given name.
///
/// Returns the live `ContainerAsync` handle. Prefer [`get_aws_config`] unless
/// you need direct access to the container (e.g., to stop it in tests).
pub async fn get_container(container_name: &str) -> ContainerAsync<GenericImage> {
    let container = GenericImage::new("motoserver/moto", "5.1.22")
        .with_wait_for(WaitFor::message_on_either_std("Running on all addresses"))
        .with_container_name(container_name)
        .with_cmd(["-H", "0.0.0.0", "-p", "5000"])
        .with_env_var("MOTO_ALLOW_NONEXISTENT_REGION", "true")
        .with_reuse(ReuseDirective::Always)
        .start()
        .await
        .expect("Failed to start Moto");
    container
}

async fn create_config(endpoint_url: &str) -> SdkConfig {
    let creds = Credentials::new("testing", "testing", None, None, "static");

    aws_config::from_env()
        .credentials_provider(creds)
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint_url)
        .load()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_container_doest_conflict_on_port_reuse_with_same_name() {
        const NAME_1: &'static str = "random-name-MOTO-1";
        const NAME_2: &'static str = "random-name-MOTO-2";

        { // CLEANUP to ensure the
            let c0 = get_container("random-name-MOTO-1").await;
            c0.stop().await.expect("Should stop");
            let c0 = get_container("random-name-MOTO-2").await;
            c0.stop().await.expect("Should stop");
        }

        let c1 = get_container(NAME_1).await;
        let c2 = get_container(NAME_1).await;
        assert_eq!(c1.id(), c2.id());
        let c3 = get_container(NAME_2).await;
        assert_ne!(c1.id(), c3.id());

        println!(
            "used port is {} & {} & {}",
            c1.get_host_port_ipv4(MOTO_PORT).await.unwrap(),
            c2.get_host_port_ipv4(MOTO_PORT).await.unwrap(),
            c3.get_host_port_ipv4(MOTO_PORT).await.unwrap()
        );
    }


    use aws_sdk_sns::Client as SnsClient;
    use aws_sdk_sqs::{Client as SqsClient, types::QueueAttributeName};

    // Helper to create a client pointing to your Moto Testcontainer
    #[tokio::test]
    async fn test_sns_publish_subscribe() {
        let run_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_micros();
        let queue_name = format!("test-queue-{}", run_id);
        let topic_name = format!("test-topic-{}", run_id);

        let config = get_aws_config("motoserver-sns-TEST-1").await;
        let sns_client = SnsClient::new(&config);
        let sqs_client = SqsClient::new(&config);

        // --- STEP 1: Create the SQS Queue ---
        let create_queue_out = sqs_client
            .create_queue()
            .queue_name(&queue_name)
            .send()
            .await
            .expect("Failed to create SQS queue");

        let attr_res = sqs_client
            .get_queue_attributes()
            .queue_url(create_queue_out.queue_url().unwrap())
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await
            .expect("Failed to fetch queue attributes");

        let queue_arn = attr_res
            .attributes().expect("Failed to fetch queue attributes")
            .get(&QueueAttributeName::QueueArn)
            .cloned()
            .expect("ARN not found in attributes");


        // --- STEP 2: Create the SNS Topic ---
        let create_topic_out = sns_client
            .create_topic()
            .name(&topic_name)
            .send()
            .await
            .expect("Failed to create topic");


        let topic_arn = create_topic_out.topic_arn().unwrap();

        // --- STEP 3: Subscribe (This will now succeed) ---
        sns_client
            .subscribe()
            .topic_arn(topic_arn)
            .protocol("sqs")
            .endpoint(queue_arn) // Pointing to the queue we just created
            .send()
            .await
            .expect("Failed to subscribe");

        // --- STEP 4: Publish ---
        let res = sns_client
            .publish()
            .topic_arn(topic_arn)
            .message("Internal Proxy Alert")
            .send()
            .await;

        if let Err(err) =  &res {
            println!("{:?}", err);
        }

        assert!(res.is_ok(), "Publishing to SNS failed");

        let message_id = res.unwrap().message_id;
        println!("Published message ID: {:?}", message_id);
    }

}
