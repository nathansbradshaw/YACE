//! Exercise the production folder bridge with real browser filesystem handles.
//! OPFS isolates fixtures from the user's selected folders and permission grants.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::{
    FileSystemCreateWritableOptions, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetDirectoryOptions, FileSystemGetFileOptions, FileSystemRemoveOptions,
    FileSystemWritableFileStream,
};
use yace_web::{
    editor::{Action, Actor, ActorKind, Request, Store},
    project_folder, song_metadata, storage,
};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn new_project_requires_an_explicit_destination() {
    use yace_web::project_destination::Choice;
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        let default = Choice::default();
        ensure!(
            matches!(default, Choice::PickFolder) == project_folder::supported(),
            "A supporting browser didn't default to the local folder picker"
        );
        ensure!(
            !matches!(default, Choice::Browser),
            "Defaulted to browser storage"
        );
        ensure!(!default.ready(), "Default bypassed folder selection");
        ensure!(
            default.save(&fixture.root).await.is_err(),
            "Unresolved default saved a project"
        );
        let unchosen = Choice::Unchosen;
        ensure!(!unchosen.ready(), "New project silently chose a location");
        ensure!(
            unchosen.save(&fixture.root).await.is_err(),
            "Unchosen destination accepted"
        );
        ensure!(
            !project_folder::linked(&fixture.root),
            "Rejected choice linked a folder"
        );
        ensure!(
            Choice::Browser.ready(),
            "Explicit browser choice is unavailable"
        );
        ensure!(
            matches!(Choice::Browser.resolve()?.await?, Some(Choice::Browser)),
            "Explicit browser choice wasn't retained"
        );
        Choice::Browser.save(&fixture.root).await?;
        ensure!(
            !project_folder::linked(&fixture.root),
            "Browser choice linked a folder"
        );
        ensure!(
            Store::new(&fixture.root).load()?.revision == 0,
            "Choosing a location changed the chart"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn new_project_folder_contains_the_initial_document_and_subsequent_edits() {
    use yace_web::project_destination::Choice;
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        let choice = Choice::Folder(fixture.folder.clone());
        ensure!(choice.ready(), "Folder choice wasn't accepted");
        choice.save(&fixture.root).await?;
        ensure!(
            project_folder::linked(&fixture.root),
            "New project isn't linked to its chosen folder"
        );
        let initial = read(&fixture.folder, "project.json").await?;
        let source = storage::read(fixture.root.join("source/notes.chart"))?;
        ensure!(
            serde_json::from_slice::<serde_json::Value>(&initial)?["revision"] == 0,
            "Initial document wasn't published"
        );
        edit(&fixture.root)?;
        yace_web::persistence::save(&fixture.root, 1, Some(0)).await?;
        ensure!(
            serde_json::from_slice::<serde_json::Value>(
                &read(&fixture.folder, "project.json").await?
            )?["revision"]
                == 1,
            "Edit wasn't autosaved to the selected folder"
        );
        storage::clear(&fixture.root);
        project_folder::open(&fixture.root, fixture.folder.clone()).await?;
        let reopened = Store::new(&fixture.root).load()?;
        ensure!(
            storage::read(fixture.root.join("source/notes.chart"))? == source,
            "Folder reopen lost the source"
        );
        ensure!(
            reopened.revision == 1 && !reopened.undo.is_empty(),
            "Folder reopen lost the edit or undo history"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn rejected_new_project_folder_never_falls_back_to_browser_storage() {
    use yace_web::project_destination::Choice;
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        replace(&fixture.folder, "existing.txt", b"keep this file").await?;
        ensure!(
            Choice::Folder(fixture.folder.clone())
                .save(&fixture.root)
                .await
                .is_err(),
            "Nonempty folder accepted"
        );
        ensure!(
            !project_folder::linked(&fixture.root),
            "Rejected folder became active"
        );
        ensure!(
            read(&fixture.folder, "existing.txt").await? == b"keep this file",
            "Existing file changed"
        );
        ensure!(
            Store::new(&fixture.root).load()?.revision == 0,
            "Prepared chart wasn't retained for retry"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn missing_working_files_cannot_replace_a_complete_browser_save() {
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        let root = &fixture.root;
        yace_web::persistence::save(root, 0, None).await?;
        storage::clear(root);
        ensure!(
            yace_web::persistence::save(root, 0, Some(0)).await.is_err(),
            "An empty snapshot was accepted"
        );
        let restored = yace_web::persistence::load(root)
            .await?
            .context("The previous durable save was lost")?;
        ensure!(
            restored.1 == 0 && Store::new(root).load()?.revision == 0,
            "Durable revision changed"
        );
        ensure!(
            storage::shared(root.join("source/notes.chart"))?.len() > 0,
            "Source chart was lost"
        );
        edit(root)?;
        yace_web::persistence::save(root, 1, Some(0)).await?;
        storage::activate_project(Some(root));
        storage::clear(root);
        ensure!(
            Store::new(root).load()?.revision == 1,
            "Cleanup released the open project"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    storage::activate_project(None);
    fixture.cleanup().await;
    result.unwrap();
}

fn error(value: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow::anyhow!("{value:?}")
}

struct Fixture {
    parent: FileSystemDirectoryHandle,
    folder: FileSystemDirectoryHandle,
    name: String,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let parent: FileSystemDirectoryHandle = JsFuture::from(
            web_sys::window()
                .context("Window")?
                .navigator()
                .storage()
                .get_directory(),
        )
        .await
        .map_err(error)?
        .unchecked_into();
        let name = format!("yace-folder-test-{}", storage::unique_id());
        let options = FileSystemGetDirectoryOptions::new();
        options.set_create(true);
        let folder = JsFuture::from(parent.get_directory_handle_with_options(&name, &options))
            .await
            .map_err(error)?
            .unchecked_into();
        let root = yace_web::files::unique_root();
        let (ini, chart) = song_metadata::blank_song("Folder test", "Synthetic fixture", 120.)?;
        storage::write(root.join("source/song.ini"), ini)?;
        storage::write(root.join("source/notes.chart"), chart)?;
        Store::new(&root).initialize(&root.join("source"), None, 4.)?;
        Ok(Self {
            parent,
            folder,
            name,
            root,
        })
    }

    async fn cleanup(self) {
        storage::clear(&self.root);
        let options = FileSystemRemoveOptions::new();
        options.set_recursive(true);
        JsFuture::from(self.parent.remove_entry_with_options(&self.name, &options))
            .await
            .expect("Remove this test's OPFS folder");
    }
}

async fn handle(folder: &FileSystemDirectoryHandle, name: &str) -> Result<FileSystemFileHandle> {
    let options = FileSystemGetFileOptions::new();
    options.set_create(true);
    Ok(
        JsFuture::from(folder.get_file_handle_with_options(name, &options))
            .await
            .map_err(error)?
            .unchecked_into(),
    )
}

async fn writable(handle: &FileSystemFileHandle) -> Result<FileSystemWritableFileStream> {
    let options = FileSystemCreateWritableOptions::new();
    js_sys::Reflect::set(&options, &"mode".into(), &"exclusive".into()).map_err(error)?;
    Ok(
        JsFuture::from(handle.create_writable_with_options(&options))
            .await
            .map_err(error)?
            .unchecked_into(),
    )
}

async fn read(folder: &FileSystemDirectoryHandle, name: &str) -> Result<Vec<u8>> {
    let handle = handle(folder, name).await?;
    let file: web_sys::File = JsFuture::from(handle.get_file())
        .await
        .map_err(error)?
        .unchecked_into();
    let buffer = JsFuture::from(file.array_buffer()).await.map_err(error)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

async fn replace(folder: &FileSystemDirectoryHandle, name: &str, bytes: &[u8]) -> Result<()> {
    let stream = writable(&handle(folder, name).await?).await?;
    JsFuture::from(stream.write_with_u8_array(bytes).map_err(error)?)
        .await
        .map_err(error)?;
    JsFuture::from(stream.close()).await.map_err(error)?;
    Ok(())
}

fn edit(root: &Path) -> Result<()> {
    let store = Store::new(root);
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Folder integration test".into(),
        },
        action: Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 192,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    })?;
    Ok(())
}

#[wasm_bindgen_test]
async fn folder_roundtrip_preserves_sources_chunked_assets_and_undo() {
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        let root = &fixture.root;
        let asset = vec![0x9d; 4 * 1024 * 1024 + 7];
        storage::write(root.join("chunked.bin"), &asset)?;
        storage::write(root.join("source/authored.lock"), b"authored companion")?;
        storage::write(root.join("window.lock"), b"disposable session lock")?;
        project_folder::attach_new(root, fixture.folder.clone(), "project.json").await?;
        ensure!(
            project_folder::linked(root),
            "The selected folder is linked"
        );
        ensure!(
            read(&fixture.folder, "chunked.bin").await? == asset,
            "Chunked asset bytes changed"
        );
        let source = storage::read(root.join("source/notes.chart"))?;
        edit(root)?;
        project_folder::save(root).await?;
        storage::clear(root);
        project_folder::open(root, fixture.folder.clone()).await?;
        let store = Store::new(root);
        let project = store.load()?;
        ensure!(
            project.revision == 1 && project.undo.len() == 1,
            "Missing saved edit or undo history"
        );
        ensure!(
            storage::read(root.join("source/notes.chart"))? == source,
            "Immutable source changed"
        );
        ensure!(
            storage::read(root.join("source/authored.lock"))? == b"authored companion",
            "Source companion missing"
        );
        ensure!(
            !storage::is_file(root.join("window.lock")),
            "Session lock was copied"
        );
        store.apply(Request {
            expected_revision: project.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Folder integration test".into(),
            },
            action: Action::Undo,
        })?;
        project_folder::save(root).await?;
        storage::clear(root);
        project_folder::open(root, fixture.folder.clone()).await?;
        ensure!(
            Store::new(root).load()?.revision == 2,
            "Undo wasn't saved to the folder"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn conflicting_document_preserves_both_disk_and_open_edits() {
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        project_folder::attach_new(&fixture.root, fixture.folder.clone(), "project.json").await?;
        edit(&fixture.root)?;
        replace(
            &fixture.folder,
            "project.json",
            b"external editor's document",
        )
        .await?;
        let failure = project_folder::save(&fixture.root)
            .await
            .expect_err("Refuse the changed disk document");
        ensure!(
            format!("{failure:#}").contains("changed outside YACE"),
            "Missing conflict explanation"
        );
        ensure!(
            read(&fixture.folder, "project.json").await? == b"external editor's document",
            "External edit overwritten"
        );
        ensure!(
            Store::new(&fixture.root).load()?.revision == 1,
            "Open edits lost"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn failed_asset_write_keeps_committed_document_and_can_retry() {
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        storage::write(fixture.root.join("new.bin"), b"old asset")?;
        project_folder::attach_new(&fixture.root, fixture.folder.clone(), "project.json").await?;
        let committed = read(&fixture.folder, "project.json").await?;
        storage::write(fixture.root.join("new.bin"), b"new asset")?;
        edit(&fixture.root)?;
        let file = handle(&fixture.folder, "new.bin").await?;
        let lock = writable(&file).await?;
        let failed = project_folder::save(&fixture.root).await.is_err();
        let unchanged = read(&fixture.folder, "project.json").await? == committed;
        JsFuture::from(lock.abort()).await.map_err(error)?;
        ensure!(failed, "Locked asset write should fail");
        ensure!(unchanged, "Published document before its assets");
        project_folder::save(&fixture.root).await?;
        ensure!(
            read(&fixture.folder, "new.bin").await? == b"new asset",
            "Retry didn't write the asset"
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&read(&fixture.folder, "project.json").await?)?;
        ensure!(saved["revision"] == 1, "Retry didn't publish the document");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}

#[wasm_bindgen_test]
async fn song_folder_opens_and_saves_its_managed_project() {
    let fixture = Fixture::new().await.unwrap();
    let result = async {
        let options = FileSystemGetDirectoryOptions::new();
        options.set_create(true);
        let private: FileSystemDirectoryHandle = JsFuture::from(
            fixture
                .folder
                .get_directory_handle_with_options(".yace", &options),
        )
        .await
        .map_err(error)?
        .unchecked_into();
        let project: FileSystemDirectoryHandle =
            JsFuture::from(private.get_directory_handle_with_options("project", &options))
                .await
                .map_err(error)?
                .unchecked_into();
        project_folder::attach_new(&fixture.root, project.clone(), "project.json").await?;
        storage::clear(&fixture.root);
        ensure!(
            project_folder::open(&fixture.root, fixture.folder.clone()).await? == "project.json",
            "Didn't find the song's .yace/project"
        );
        edit(&fixture.root)?;
        project_folder::save(&fixture.root).await?;
        let saved: serde_json::Value =
            serde_json::from_slice(&read(&project, "project.json").await?)?;
        ensure!(
            saved["revision"] == 1,
            "Saved outside the song's managed project"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    fixture.cleanup().await;
    result.unwrap();
}
