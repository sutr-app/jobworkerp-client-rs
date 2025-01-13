use crate::jobworkerp::data::{JobResultData, Runner, Worker};
use crate::jobworkerp::service::WorkerNameRequest;
use crate::{
    client::JobworkerpClient,
    jobworkerp::{
        data::{RunnerData, RunnerId},
        service::job_request,
    },
};
use anyhow::{Context, Result};
use command_utils::protobuf::ProtobufDescriptor;
use prost::Message;
use prost_reflect::{DynamicMessage, MessageDescriptor};
use serde_json::Deserializer;

pub struct JobworkerpProto {}

impl JobworkerpProto {
    pub async fn find_worker_runner_settings_descriptors(
        client: &JobworkerpClient,
        runner_id: RunnerId,
    ) -> Result<Option<MessageDescriptor>> {
        let response = client
            .runner_client()
            .await
            .find(runner_id)
            .await?
            .into_inner()
            .data
            .and_then(|r| r.data);
        if let Some(runner_data) = response {
            let runner_settings_descriptor =
                Self::parse_runner_settings_schema_descriptor(&runner_data)?;
            Ok(runner_settings_descriptor)
        } else {
            Err(anyhow::anyhow!(
                "runner not found: runner_id = {}",
                runner_id.value
            ))
        }
    }
    pub async fn find_runner_descriptors(
        client: &JobworkerpClient,
        runner_id: RunnerId,
    ) -> Result<(
        Option<MessageDescriptor>,
        Option<MessageDescriptor>,
        Option<MessageDescriptor>,
    )> {
        let response = client
            .runner_client()
            .await
            .find(runner_id)
            .await?
            .into_inner()
            .data
            .and_then(|r| r.data);
        if let Some(runner_data) = response {
            let runner_settings_descriptor =
                Self::parse_runner_settings_schema_descriptor(&runner_data)?;
            let args_descriptor = Self::parse_job_args_schema_descriptor(&runner_data)?;
            let result_descriptor = Self::parse_result_schema_descriptor(&runner_data)?;
            Ok((
                runner_settings_descriptor,
                args_descriptor,
                result_descriptor,
            ))
        } else {
            Err(anyhow::anyhow!(
                "runner not found: runner_id = {}",
                runner_id.value
            ))
        }
    }
    pub async fn find_runner_descriptors_by_worker(
        client: &JobworkerpClient,
        worker: job_request::Worker,
    ) -> Result<(
        Option<MessageDescriptor>,
        Option<MessageDescriptor>,
        Option<MessageDescriptor>,
    )> {
        let runner_id = match worker.clone() {
            job_request::Worker::WorkerId(id) => client.worker_client().await.find(id).await?,
            job_request::Worker::WorkerName(name) => client
                .worker_client()
                .await
                .find_by_name(WorkerNameRequest { name })
                .await
                .context("failed to find worker by name (find_runner_descriptors_by_worker)")?,
        }
        .into_inner()
        .data
        .and_then(|r| r.data.and_then(|r| r.runner_id))
        .ok_or(anyhow::anyhow!("runner not found: worker: {:?}", &worker))?;
        Self::find_runner_descriptors(client, runner_id).await
    }
    pub fn json_to_message(descriptor: MessageDescriptor, json_str: &str) -> Result<Vec<u8>> {
        let mut deserializer = Deserializer::from_str(json_str);
        let dynamic_message = DynamicMessage::deserialize(descriptor, &mut deserializer)?;
        deserializer.end()?;
        Ok(dynamic_message.encode_to_vec())
    }
    pub fn parse_runner_settings_schema_descriptor(
        runner_data: &RunnerData,
    ) -> Result<Option<MessageDescriptor>> {
        if runner_data.runner_settings_proto.is_empty() {
            Ok(None)
        } else {
            let descriptor = ProtobufDescriptor::new(&runner_data.runner_settings_proto)?;
            descriptor
                .get_messages()
                .first()
                .map(|m| Some(m.clone()))
                .ok_or_else(|| anyhow::anyhow!("message not found"))
        }
    }
    pub fn parse_job_args_schema_descriptor(
        runner_data: &RunnerData,
    ) -> Result<Option<MessageDescriptor>> {
        if runner_data.job_args_proto.is_empty() {
            Ok(None)
        } else {
            let descriptor = ProtobufDescriptor::new(&runner_data.job_args_proto)?;
            descriptor
                .get_messages()
                .first()
                .map(|m| Some(m.clone()))
                .ok_or_else(|| anyhow::anyhow!("message not found"))
        }
    }
    pub fn parse_result_schema_descriptor(
        runner_data: &RunnerData,
    ) -> Result<Option<MessageDescriptor>> {
        if let Some(proto) = &runner_data.result_output_proto {
            if proto.is_empty() {
                Ok(None)
            } else {
                let descriptor = ProtobufDescriptor::new(proto)?;
                descriptor
                    .get_messages()
                    .first()
                    .map(|m| Some(m.clone()))
                    .ok_or_else(|| anyhow::anyhow!("message not found"))
            }
        } else {
            Ok(None)
        }
    }
    pub async fn resolve_result_descriptor(
        client: &crate::client::JobworkerpClient,
        worker_name: &str,
    ) -> Option<MessageDescriptor> {
        if let Some(Worker {
            id: Some(_wid),
            data: Some(wdata),
        }) = client
            .worker_client()
            .await
            .find_by_name(WorkerNameRequest {
                name: worker_name.to_string(),
            })
            .await
            .unwrap()
            .into_inner()
            .data
        {
            tracing::debug!("worker {} found: {:#?}", worker_name, &wdata);
            if let Some(Runner {
                id: Some(_sid),
                data: Some(sdata),
            }) = client
                .runner_client()
                .await
                .find(wdata.runner_id.unwrap())
                .await
                .unwrap()
                .into_inner()
                .data
            {
                tracing::debug!("worker {}  found: {:#?}", worker_name, &sdata);
                sdata.result_output_proto.and_then(|p| {
                    if p.trim().is_empty() {
                        None
                    } else {
                        tracing::debug!("protobuf output proto decode: {:#?}", &p);
                        ProtobufDescriptor::new(&p)
                            .inspect_err(|e| tracing::warn!("protobuf decode error: {:#?}", e))
                            .ok()
                            .and_then(|d| d.get_messages().first().cloned())
                    }
                })
            } else {
                tracing::warn!("runner not found: {:#?}", &wdata.runner_id);
                None
            }
        } else {
            tracing::warn!("worker not found: {:#?}", &worker_name);
            None
        }
    }
    pub async fn resolve_result_output_to_string(
        client: &crate::client::JobworkerpClient,
        worker_name: &str,
        result_data: &JobResultData,
    ) -> Result<String> {
        let output_text: String = match result_data.output.as_ref() {
            Some(output) if !output.items.is_empty() && !output.items[0].is_empty() => {
                let result_proto =
                    JobworkerpProto::resolve_result_descriptor(client, worker_name).await;
                if let Some(proto) = result_proto.as_ref() {
                    let mut output_text = "".to_string();
                    for item in output.items.iter() {
                        if item.is_empty() {
                            continue;
                        }
                        tracing::debug!(
                            "protobuf decode item: {}, worker: {}",
                            item.len(),
                            worker_name
                        );
                        match ProtobufDescriptor::get_message_from_bytes(
                            proto.clone(),
                            item.as_slice(),
                        ) {
                            Ok(mes) => {
                                output_text +=
                                    ProtobufDescriptor::dynamic_message_to_string(&mes, false)
                                        .as_str();
                            }
                            Err(e) => {
                                output_text += format!("protobuf decode error: {:#?}", e).as_str();
                            }
                        }
                    }
                    output_text
                } else if !output.items.is_empty() && !output.items[0].is_empty() {
                    tracing::debug!("empty proto (means raw bytes): worker={}", worker_name);
                    output
                        .items
                        .iter()
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    tracing::debug!("empty proto, empty item: worker={}", worker_name);
                    "".to_string()
                }
            }
            Some(output) => output
                .items
                .iter()
                .map(|s| String::from_utf8_lossy(s))
                .collect::<Vec<_>>()
                .join("\n"),
            None => "".to_string(),
        };
        Ok(output_text)
    }
}
