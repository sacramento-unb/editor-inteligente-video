use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

pub fn safe_filename(input: &str) -> String {
    let input = input.replace(['/', '\\'], "_");
    let stem = input
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ')
        })
        .collect::<String>()
        .trim()
        .to_string();
    if stem.is_empty() {
        "upload.bin".into()
    } else {
        stem
    }
}

pub async fn project_dir(root: &Path, project_id: &str) -> anyhow::Result<PathBuf> {
    let dir = root.join("projects").join(project_id);
    for child in [
        "original",
        "proxy",
        "audio",
        "analysis",
        "transcription",
        "output",
        "material",
    ] {
        fs::create_dir_all(dir.join(child)).await?;
    }
    Ok(dir)
}

pub async fn write_upload(
    root: &Path,
    project_id: &str,
    original_name: &str,
    mut field: axum::extract::multipart::Field<'_>,
    max_bytes: u64,
) -> anyhow::Result<(String, u64, String)> {
    let dir = project_dir(root, project_id).await?;
    let filename = format!("{}_{}", Uuid::new_v4(), safe_filename(original_name));
    let relative = format!("projects/{project_id}/original/{filename}");
    let destination = root.join(&relative);
    let temporary = destination.with_extension("uploading");
    let mut file = fs::File::create(&temporary)
        .await
        .context("criar arquivo temporário")?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;

    while let Some(chunk) = field.chunk().await.context("ler upload")? {
        size = size.saturating_add(chunk.len() as u64);
        if size > max_bytes {
            let _ = fs::remove_file(&temporary).await;
            bail!("upload excede o limite de {} bytes", max_bytes);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.context("gravar upload")?;
    }
    file.flush().await?;
    drop(file);
    fs::rename(&temporary, &destination)
        .await
        .context("finalizar upload")?;
    let _ = dir;
    Ok((relative, size, format!("{:x}", hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::safe_filename;

    #[test]
    fn sanitizes_paths_and_keeps_extension() {
        assert_eq!(
            safe_filename("../aula\\parte 01.mp4"),
            ".._aula_parte 01.mp4"
        );
        assert_eq!(safe_filename("\0"), "upload.bin");
    }
}
