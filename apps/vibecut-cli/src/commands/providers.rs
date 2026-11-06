use crate::arguments::utf8;
use crate::exit_codes::CliError;
use crate::output::Output;
use engine::subtitles::{CatalogOperation, ProviderOption};
use engine::{clip_suggest, subtitles};
use serde_json::{Value, json};
use std::ffi::OsString;

pub(crate) fn providers(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut operation = CatalogOperation::Transcribe;
    let mut language = None;
    while let Some(arg) = args.next() {
        match utf8(&arg, "providers flag")? {
            "--operation" => {
                let raw = args.next().ok_or_else(|| {
                    CliError::Usage("--operation requires transcribe or translate".into())
                })?;
                let value = utf8(&raw, "operation")?;
                operation = match value {
                    "transcribe" => CatalogOperation::Transcribe,
                    "translate" => CatalogOperation::Translate,
                    "clips" => CatalogOperation::SuggestClips,
                    _ => {
                        return Err(CliError::Usage(
                            "--operation must be transcribe, translate or clips".into(),
                        ));
                    }
                };
            }
            "--language" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--language requires a tag".into()))?;
                let value = utf8(&raw, "language")?;
                language = Some(subtitles::parse_language(value)?);
            }
            other => {
                return Err(CliError::Usage(format!("unknown providers flag: {other}")));
            }
        }
    }
    if operation == CatalogOperation::SuggestClips {
        let options = clip_suggest::suggestion_options();
        let value: Vec<Value> = options
            .iter()
            .map(|option| {
                json!({
                    "id": option.id,
                    "label": option.label,
                    "operation": "suggest_clips",
                    "location": if option.local { "local" } else { "remote" },
                    "models": option.models,
                    "policy_url": option.policy_url,
                })
            })
            .collect();
        let human = options
            .iter()
            .map(|option| {
                format!(
                    "{}\t{}\t{}\t{}",
                    option.id,
                    option.label,
                    option.models.join(","),
                    if option.local { "local" } else { "remote" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Output {
            silent: false,
            exit: 0,
            value: json!({"providers": value}),
            human,
        });
    }
    let options = subtitles::provider_options_for(operation, language.as_ref());
    let value: Vec<Value> = options.iter().map(provider_option_json).collect();
    let human = options
        .iter()
        .map(|option| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                option.id,
                option.label,
                option.models.join(","),
                option.languages.join(","),
                if option.local { "local" } else { "remote" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Output {
        silent: false,
        exit: 0,
        value: json!({"providers": value}),
        human,
    })
}
fn provider_option_json(option: &ProviderOption) -> Value {
    json!({
        "id": option.id,
        "label": option.label,
        "operation": match option.operation {
            CatalogOperation::Transcribe => "transcribe",
            CatalogOperation::Translate => "translate",
            CatalogOperation::SuggestClips => "suggest_clips",
        },
        "location": if option.local { "local" } else { "remote" },
        "models": option.models,
        "languages": option.languages,
        "target_languages": option.target_languages,
        "native_timing": option.native_timing,
    })
}
