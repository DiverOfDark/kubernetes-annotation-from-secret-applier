use kube::{Client, Api, runtime::controller::{Controller}, api::{Patch, PatchParams}, Error};
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::api::core::v1::Secret;
use serde_json::json;
use futures::{StreamExt};
use tracing::{info, error};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use k8s_openapi::Metadata;
use kube::runtime::watcher;
use kube::runtime::controller::Action;
use kube::runtime::reflector::Lookup;

#[derive(Clone)]
struct OperatorContext {
    client: Client,
}

const SECRET_ANNOTATION: &str = "kirillorlov.pro/annotationsFromSecretName";
const SECRET_ANNOTATION_STATE: &str = "kirillorlov.pro/annotationsFromSecretState";

async fn reconcile(ingress: Arc<Ingress>, ctx: Arc<OperatorContext>) -> Result<Action, Error> {
    // collect all ingresses and save track to them: namespace + name + reference to secret(name)
    // for each ingress -> save namespace and ensure we watch it for secrets (ref counting here?)
    // for each reconcile of ingress -> trigger sync of that ingress
    // for each change on ingress -> trigger sync of that ingress
    // for each change on watched secret -> discover related ingress and reconcile them
    //

    let annotations = ingress.metadata().annotations.as_ref();
    if !annotations.is_some() {
        return Ok(Action::await_change());
    }

    let annotations = annotations.unwrap();
    if !annotations.contains_key(SECRET_ANNOTATION) {
        return Ok(Action::await_change());
    }

    let secret_name = annotations.get(SECRET_ANNOTATION).unwrap();

    let current_namespace = ingress.metadata().namespace.as_ref().unwrap();

    let secret_api = Api::<Secret>::namespaced(ctx.client.clone(), current_namespace);

    let secret = match secret_api.get(secret_name).await {
        Ok(secret) => secret,
        Err(e) => {
            error!("Failed to get Secret: {:?}", e);
            return Err(e.into());
        }
    };

    let api = Api::<Ingress>::namespaced(ctx.client.clone(), &current_namespace);

    match apply(api, ingress, secret).await {
        Ok(_) => Ok(Action::requeue(Duration::from_secs(300))), // Requeue after 5 minutes
        Err(e) => Err(e),
    }
}

async fn apply(api: Api<Ingress>, ingress: Arc<Ingress>, secret: Secret) -> Result<i32, Error> {
    let mut replacements = BTreeMap::new();
    if secret.data.is_some() {
        let data = secret.data.as_ref().unwrap();

        for (k, v) in data {
            let str = String::from_utf8(v.0.clone()).unwrap();
            replacements.insert(k.clone(), str.clone());
        }
    }
    if secret.string_data.is_some() {
        let string_data = secret.string_data.as_ref().unwrap();
        for (k, v) in string_data {
            replacements.insert(k.clone(), v.clone());
        }
    }

    let mut updated_annotations = BTreeMap::new();
    let mut old_values = BTreeMap::new();

    let mut old_items : BTreeMap<String, String> = BTreeMap::new();
    if ingress.metadata.annotations.as_ref().unwrap().contains_key(SECRET_ANNOTATION_STATE) {
        let old_values_string = ingress.metadata.annotations.as_ref().unwrap().get(SECRET_ANNOTATION_STATE).unwrap();
        let result = serde_json::from_str(old_values_string);
        if result.is_ok() {
            old_items = result.unwrap();
        }
    }

    for (key, value) in ingress.metadata().annotations.as_ref().unwrap() {
        if key == "kubectl.kubernetes.io/last-applied-configuration" {
            continue;
        }
        if key == SECRET_ANNOTATION_STATE {
            continue;
        }

        for (replacement_key, replacement_value) in &replacements {
            let x = String::from("$") + replacement_key.as_str() + "$";

            let mut original_value = value;

            if old_items.contains_key(key) {
                original_value = old_items.get(key).unwrap();
            }

            if original_value.as_str().contains(x.as_str()) {
                let replaced_value = original_value.replace(x.as_str(), replacement_value.as_str());

                if replaced_value != value.as_str() {
                    updated_annotations.insert(key.clone(), replaced_value);
                    old_values.insert(key.clone(), original_value.clone());
                }
            }
        }
    }

    if updated_annotations.len() == 0 {
        return Ok(0);
    }

    updated_annotations.insert(String::from(SECRET_ANNOTATION_STATE), serde_json::to_string(&old_values).unwrap());

    // Update the Ingress with new annotations
    let patch = json!({
        "metadata": {
            "annotations": updated_annotations,
        }
    });

    let ingress_name = String::from(ingress.name().clone().unwrap());
    match api.patch(ingress_name.as_ref(), &PatchParams::apply("my-operator"), &Patch::Merge(&patch)).await {
        Ok(_) => {
            info!("Patched Ingress {} with new annotations", ingress_name);
            Ok(1)
        }
        Err(e) => {
            error!("Failed to patch Ingress: {:?}", e);
            Err(e.into())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let client = Client::try_default().await?;

    let context = Arc::new(OperatorContext {
        client: client.clone()
    });

    let ingress_api = Api::<Ingress>::all(client.clone());

    let controller = Controller::new(ingress_api, watcher::Config::default())
        .run(reconcile, error_policy, context)
        .for_each(|reconciliation| async move {
            match reconciliation {
                Ok(resource) => info!("Reconciled {:?}", resource),
                Err(e) => error!("Reconciliation failed: {:?}", e),
            }
        });

    controller.await;
    Ok(())
}

fn error_policy(_ingress: Arc<Ingress>, _error: &Error, _ctx: Arc<OperatorContext>) -> Action {
    Action::requeue(Duration::from_secs(60)) // Requeue after 1 minute
}
