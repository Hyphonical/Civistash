use std::path::Path;

use hf_hub::repository::AddSource;
use hf_hub::HFClient;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum UploadError {
	#[error("tarball not found at {0}")]
	MissingTarball(String),

	#[error("tarball path has no filename: {0}")]
	NoFilename(String),

	#[error("invalid repo id `{0}` (expected `namespace/name`)")]
	InvalidRepoId(String),

	#[error("Hugging Face: {0}")]
	Hub(#[from] hf_hub::HFError),
}

/// Upload a tarball to a Hugging Face dataset repo. The file lands at
/// the repo root with its basename as the path.
///
/// `token`, when `Some`, takes precedence over `HF_TOKEN`. When `None`,
/// `hf-hub`'s default token resolution runs (`HF_TOKEN` env, then
/// `~/.cache/huggingface/token`).
pub async fn upload_to_hf(
	tarball: &Path,
	repo_id: &str,
	token: Option<&str>,
) -> Result<(), UploadError> {
	if !tarball.exists() {
		return Err(UploadError::MissingTarball(tarball.display().to_string()));
	}

	let filename = tarball
		.file_name()
		.and_then(|n| n.to_str())
		.ok_or_else(|| UploadError::NoFilename(tarball.display().to_string()))?
		.to_owned();

	let (namespace, name) = repo_id
		.split_once('/')
		.ok_or_else(|| UploadError::InvalidRepoId(repo_id.to_owned()))?;

	info!(repo = repo_id, tarball = %tarball.display(), "uploading to Hugging Face");

	let client = match token {
		Some(t) => HFClient::builder().token(t).build()?,
		None => HFClient::new()?,
	};

	let commit = client
		.dataset(namespace, name)
		.upload_file()
		.source(AddSource::file(tarball))
		.path_in_repo(&filename)
		.send()
		.await?;

	info!(
		oid = commit.commit_oid.as_deref().unwrap_or("(none)"),
		repo = repo_id,
		"HF commit created"
	);

	Ok(())
}
