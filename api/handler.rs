use data_encoding::HEXLOWER;
use http::{header, Method};
use octocrab::{
    models::{workflows::Status, RunId},
    params::actions::ArchiveFormat,
    OctocrabBuilder,
};
use ring::digest::{Context, Digest, SHA256};
use std::io::{self, Cursor, Read};
use vercel_runtime::{run, Body, Error, Request, Response, StatusCode};

const REPO_OWNER: &str = "zihadmahiuddin";
const REPO: &str = "osus-proxy";

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(handler).await
}

pub async fn handler(req: Request) -> Result<Response<Body>, Error> {
    let gh_username = std::env::var("GH_USERNAME")?;
    let gh_token = std::env::var("GH_TOKEN")?;

    let octocrab = OctocrabBuilder::new()
        .basic_auth(gh_username, gh_token)
        .build()?;

    let workflow_runs = octocrab
        .workflows(REPO_OWNER, REPO)
        .list_all_runs()
        .send()
        .await?;

    let run_ids = workflow_runs
        .items
        .iter()
        .filter_map(|x| {
            if x.status == "completed" {
                Some(x.id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut successful_run_id: Option<RunId> = None;
    for run_id in run_ids {
        let jobs = octocrab
            .workflows(REPO_OWNER, REPO)
            .list_jobs(run_id)
            .send()
            .await?;
        if jobs.items.iter().all(|x| x.status == Status::Completed) {
            successful_run_id = Some(run_id);
            break;
        }
    }

    let Some(run_id) = successful_run_id else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::Empty)?);
    };

    let artifacts = octocrab
        .actions()
        .list_workflow_run_artifacts(REPO_OWNER, REPO, run_id)
        .send()
        .await?;

    let Some(artifact_values) = artifacts.value else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::Empty)?);
    };

    let Some(non_expired_artifact) = artifact_values.items.iter().find(|x| !x.expired) else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::Empty)?);
    };

    let artifact_zip = octocrab
        .actions()
        .download_artifact(
            REPO_OWNER,
            REPO,
            non_expired_artifact.id,
            ArchiveFormat::Zip,
        )
        .await?;

    let response = Response::builder()
        .header(header::CONTENT_LENGTH, artifact_zip.len())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}.zip\"", non_expired_artifact.name),
        )
        .header(
            "X-Content-Hash",
            format!(
                "sha256-{}",
                HEXLOWER.encode(sha256_digest(Cursor::new(&artifact_zip))?.as_ref())
            ),
        )
        .body(if req.method() == &Method::GET {
            Body::Binary(artifact_zip.to_vec())
        } else {
            Body::Empty
        })?;

    Ok(response)
}

fn sha256_digest<R: Read>(mut reader: R) -> io::Result<Digest> {
    let mut context = Context::new(&SHA256);
    let mut buffer = [0; 1024];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }

    Ok(context.finish())
}
