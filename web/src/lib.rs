pub mod acoustic_alignment;
#[path = "../../src/analysis_validation.rs"]
pub mod analysis_validation;
mod archive_blob;
#[path = "../../src/archive_codec.rs"]
pub mod archive_codec;
mod archive_import;
pub mod audio_decode;
mod audio_identity;
mod audio_resample;
#[path = "../../src/bass_analysis.rs"]
pub mod bass_analysis;
#[path = "../../src/chart.rs"]
pub mod chart;
#[path = "../../src/chart_library.rs"]
pub mod chart_library;
#[path = "../../src/clean.rs"]
pub mod clean;
#[path = "../../src/clean_transcript.rs"]
pub mod clean_transcript;
#[path = "../../src/collaboration.rs"]
pub mod collaboration;
#[path = "../../src/collaboration_exchange.rs"]
pub mod collaboration_exchange;
#[path = "../../src/compact_backup.rs"]
pub mod compact_backup;
pub mod dance_archive;
#[path = "../../src/detection_settings.rs"]
pub mod detection_settings;
#[path = "../../src/difficulty_links.rs"]
pub mod difficulty_links;
#[path = "../../src/drum_articulation.rs"]
pub mod drum_articulation;
#[path = "../../src/drum_dynamics.rs"]
pub mod drum_dynamics;
#[path = "../../src/drum_layout.rs"]
pub mod drum_layout;
#[path = "../../src/drum_parts.rs"]
pub mod drum_parts;
#[path = "../../src/drum_ranges.rs"]
pub mod drum_ranges;
#[path = "../../src/editor.rs"]
pub mod editor;
#[path = "../../src/editor_validation.rs"]
pub mod editor_validation;
#[path = "../../src/export_options.rs"]
pub mod export_options;
#[path = "../../src/export_state.rs"]
pub mod export_state;
pub mod files;
#[path = "../../src/game_export.rs"]
pub mod game_export;
#[path = "../../src/gameplay_view.rs"]
pub mod gameplay_view;
#[path = "../../src/generated_rules.rs"]
pub mod generated_rules;
pub mod generation;
#[path = "../../src/generation_build.rs"]
pub mod generation_build;
#[path = "../../src/generation_feedback.rs"]
pub mod generation_feedback;
#[path = "../../src/generation_protocol.rs"]
pub mod generation_protocol;
pub mod github_api;
#[path = "../../src/github_reviews.rs"]
pub mod github_reviews;
#[path = "../../src/guitar_attacks.rs"]
pub mod guitar_attacks;
pub mod http;
#[path = "../../src/song_import.rs"]
pub mod import_metadata;
#[path = "../../src/key_ranges.rs"]
pub mod key_ranges;
#[path = "../../src/keyboard_audio.rs"]
pub mod keyboard_audio;
#[path = "../../src/keyboard_detection.rs"]
pub mod keyboard_detection;
pub mod keyboard_separation;
#[path = "../../src/keyboard_separation_asset.rs"]
pub mod keyboard_separation_asset;
#[path = "../../src/keyboard_separation_dsp.rs"]
pub mod keyboard_separation_dsp;
#[path = "../../src/keyboard_voicing.rs"]
pub mod keyboard_voicing;
#[path = "../../src/keys.rs"]
pub mod keys;
#[path = "../../src/kit_audition.rs"]
pub mod kit_audition;
pub mod larsnet;
#[path = "../../src/larsnet_receipt.rs"]
pub mod larsnet_receipt;
#[cfg(feature = "lifetime-fixture")]
#[path = "../tests/support/project_lifetime.rs"]
mod lifetime_fixture;
#[path = "../../src/lyric_language.rs"]
pub mod lyric_language;
#[path = "../../src/mapping.rs"]
pub mod mapping;
#[path = "../../src/media_clock.rs"]
mod media_clock;
#[path = "../../src/midi.rs"]
pub mod midi;
pub mod model_assets;
pub mod models;
#[path = "../../src/muscriptor.rs"]
pub mod muscriptor;
#[path = "../../src/note_groups.rs"]
pub mod note_groups;
#[path = "../../src/note_types.rs"]
pub mod note_types;
#[path = "../../src/octave_adapter.rs"]
pub mod octave_adapter;
pub mod onnx;
pub mod part_analysis;
#[path = "../../src/parts.rs"]
pub mod parts;
pub mod persistence;
#[path = "../../src/piano_model.rs"]
pub mod piano_model;
pub mod pipeline;
mod playback_activity;
#[path = "../../src/pro_guitar.rs"]
pub mod pro_guitar;
#[path = "../../src/pro_keys.rs"]
pub mod pro_keys;
pub mod project_destination;
pub mod project_folder;
mod project_integrity;
#[path = "../../src/project_portability.rs"]
pub mod project_portability;
#[path = "../../src/project_recording.rs"]
pub mod project_recording;
pub mod project_recovery;
#[path = "../../src/reanalysis.rs"]
pub mod reanalysis;
#[path = "../../src/recent_projects.rs"]
pub mod recent_projects;
mod recognition;
#[path = "../../src/recording_metadata.rs"]
pub mod recording_metadata;
pub mod recording_tags;
#[path = "../../src/refine.rs"]
pub mod refine;
#[path = "../../src/review.rs"]
pub mod review;
#[path = "../../src/review_regions.rs"]
pub mod review_regions;
#[path = "../../src/review_repository.rs"]
pub mod review_repository;
#[path = "../../src/rmvpe_frontend.rs"]
pub mod rmvpe_frontend;
#[path = "../../src/sections.rs"]
pub mod sections;
pub mod separation;
#[path = "../../src/six_fret.rs"]
pub mod six_fret;
#[path = "../../src/solos.rs"]
pub mod solos;
#[path = "../../src/song_files.rs"]
pub mod song_files;
pub mod song_import;
#[path = "../../src/song_layout.rs"]
pub mod song_layout;
#[path = "../../src/song_metadata.rs"]
pub mod song_metadata;
#[path = "../../src/star_power.rs"]
pub mod star_power;
pub mod stem_import;
#[path = "../../src/stem_review.rs"]
pub mod stem_review;
pub mod storage;
pub mod studio;
#[path = "../../src/sync_track.rs"]
pub mod sync_track;
mod synth;
#[path = "../../src/tempo_analysis.rs"]
pub mod tempo_analysis;
#[path = "../../src/timeline.rs"]
pub mod timeline;
#[path = "../../src/timeline_clock.rs"]
pub mod timeline_clock;
#[path = "../../src/timing.rs"]
pub mod timing;
#[path = "../../src/track_names.rs"]
pub mod track_names;
pub mod transcription;
mod unload;
#[path = "../../src/vocal_acoustic_draft.rs"]
pub mod vocal_acoustic_draft;
#[path = "../../src/vocal_activity.rs"]
pub mod vocal_activity;
#[path = "../../src/vocal_adaptive.rs"]
pub mod vocal_adaptive;
#[path = "../../src/vocal_adaptive_planner.rs"]
pub mod vocal_adaptive_planner;
#[path = "../../src/vocal_alignment.rs"]
pub mod vocal_alignment;
#[path = "../../src/vocal_generation.rs"]
pub mod vocal_generation;
#[path = "../../src/vocal_harmonies.rs"]
pub mod vocal_harmonies;
pub mod vocal_inference;
#[path = "../../src/vocal_percussion.rs"]
pub mod vocal_percussion;
#[path = "../../src/vocal_pitch.rs"]
pub mod vocal_pitch;
#[path = "../../src/vocal_pitch_analysis.rs"]
pub mod vocal_pitch_analysis;
#[path = "../../src/vocal_pitch_tracking.rs"]
pub mod vocal_pitch_tracking;
#[path = "../../src/vocal_posterior.rs"]
pub mod vocal_posterior;
#[path = "../../src/vocal_review.rs"]
pub mod vocal_review;
#[path = "../../src/vocal_rmvpe.rs"]
pub mod vocal_rmvpe;
#[path = "../../src/vocal_section_analysis.rs"]
pub mod vocal_section_analysis;
#[path = "../../src/vocal_section_audio.rs"]
pub mod vocal_section_audio;
#[path = "../../src/vocal_singing.rs"]
pub mod vocal_singing;
#[path = "../../src/vocal_singing_tracking.rs"]
pub mod vocal_singing_tracking;
#[path = "../../src/vocals.rs"]
pub mod vocals;
#[path = "../../src/waveform.rs"]
pub mod waveform;
pub mod worker;
pub mod workspace_recovery;
mod workspace_target;

#[cfg(not(feature = "browser-tests"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    if web_sys::window().is_some() {
        #[cfg(feature = "lifetime-fixture")]
        wasm_bindgen_futures::spawn_local(async {
            lifetime_fixture::prepare()
                .await
                .expect("Prepare disposable lifecycle fixture");
            dioxus::launch(studio::App);
        });
        #[cfg(not(feature = "lifetime-fixture"))]
        dioxus::launch(studio::App);
    }
}
pub use yace_stepmania as stepmania;
