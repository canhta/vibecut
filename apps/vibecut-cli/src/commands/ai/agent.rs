//! Runs one agent turn over a project and prints its event stream.
use crate::args::{CliError, parse_f64, parse_u32, utf8};
use crate::output::Output;
use crate::session::{Context, Edit, History, WritePolicy, cli_runner, run_leased};
use agent::{
    AgentEvent, CancelFlag, Provider, TurnEnd, TurnLimits, TurnRequest, new_conversation,
    open_model, run_turn,
};
use ai::Money;
use ai::text::conversation::{
    AgentChatConsent, AgentChatLimits, AgentChatScope, DataKind, disclosure,
};
use engine::control::Error as EngineError;
use engine::providers::Credentials;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;

const DEFAULT_SPEND_USD: f64 = 1.0;
const MAX_SPEND_USD: f64 = 100.0;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        return Err(usage());
    };
    if project
        .to_str()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        return Ok(usage_output());
    }
    let project_path = PathBuf::from(project);
    let Some(prompt) = args.next() else {
        return Err(usage());
    };
    let prompt = utf8(&prompt, "prompt")?.to_owned();
    let mut apply = false;
    let mut provider = "anthropic".to_owned();
    let mut model: Option<String> = None;
    let mut spend = DEFAULT_SPEND_USD;
    let mut max_steps = TurnLimits::default().max_steps;
    let mut kinds = BTreeSet::from([DataKind::ProjectSummary]);
    let mut yes_upload = false;
    while let Some(flag) = args.next() {
        match utf8(&flag, "agent flag")? {
            "--apply" => apply = true,
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "agent provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "agent model",
                    )?
                    .to_owned(),
                );
            }
            "--max-spend" => spend = parse_f64(args.next(), "--max-spend")?,
            "--max-steps" => max_steps = parse_u32(args.next(), "--max-steps")?,
            "--share-clip-names" => {
                kinds.insert(DataKind::ClipNames);
            }
            "--share-transcript" => {
                kinds.insert(DataKind::TranscriptText);
            }
            "--yes-upload" => yes_upload = true,
            "--help" | "-h" => return Ok(usage_output()),
            other => return Err(CliError::Usage(format!("unknown agent flag: {other}"))),
        }
    }
    if !(spend > 0.0 && spend <= MAX_SPEND_USD) {
        return Err(CliError::Usage(
            "--max-spend must be above 0 and at most 100 USD".into(),
        ));
    }
    let provider = Provider::parse(&provider)
        .ok_or_else(|| CliError::Usage("--provider must be anthropic or openai".into()))?;
    let credentials = Credentials::from_env();
    let key = match provider {
        Provider::Anthropic => credentials.anthropic(),
        Provider::OpenAi => credentials.openai(),
    }
    .map_err(|error| CliError::Engine(error.into()))?;
    let chat = open_model(provider, model.as_deref(), key).map_err(agent_error)?;

    let limit = Money::new("USD", (spend * 1_000_000.0).round() as u64).map_err(ai_error)?;
    let scope = AgentChatScope {
        provider: chat.config().clone(),
        kinds,
        limit,
    };
    let limits = AgentChatLimits::default();
    print_disclosure(
        &disclosure(chat.as_ref(), &scope, &limits).map_err(ai_error)?,
        yes_upload,
    );
    if !yes_upload {
        return Err(CliError::Rejected(
            "consent refused; pass --yes-upload to authorize this exact scope",
        ));
    }
    let consent = AgentChatConsent::grant(scope).map_err(ai_error)?;
    let mut conversation = new_conversation(chat, &consent, limits).map_err(agent_error)?;
    let mut runner = cli_runner(context.budget()?)?;

    let saved = run_leased(
        &project_path,
        History::Single,
        WritePolicy::Changed,
        |session| {
            let turn = run_turn(
                TurnRequest {
                    session: &*session,
                    prompt: &prompt,
                    conversation: &mut conversation,
                    runner: &mut runner,
                    limits: TurnLimits { max_steps },
                    cancel: CancelFlag::new(),
                },
                &mut |event: AgentEvent| print_line(&event),
            )
            .map_err(agent_error)?;
            let end = turn.end;
            let refusals = turn.refusals;
            let changes = turn.fork.summary();
            let commands: Vec<String> = turn
                .fork
                .commands()
                .map(|command| command.id().to_owned())
                .collect();
            let mut applied = false;
            if apply && end == TurnEnd::Done && !turn.fork.is_empty() {
                session
                    .apply_fork(turn.fork)
                    .map_err(|error| CliError::Engine(error.into()))?;
                applied = true;
            }
            Ok(Edit {
                value: (end, refusals, changes, commands, applied),
                changed: applied,
            })
        },
    );
    runner.shutdown();
    let saved = saved?;
    let (end, refusals, changes, commands, applied) = saved.value;
    let frame = json!({
        "event": "result",
        "status": match end {
            TurnEnd::Done => json!("done"),
            TurnEnd::Stopped(reason) => json!({"stopped": reason}),
        },
        "applied": applied,
        "saved": saved.receipt.is_some(),
        "commands": commands,
        "changes": changes,
        "refusals": refusals,
        "spent_micros": conversation.ledger().spent_micros(),
    });
    print_line(&frame);
    Ok(Output::new(
        frame,
        String::new(),
        true,
        if end == TurnEnd::Done { 0 } else { 5 },
    ))
}

fn print_line(value: &impl serde::Serialize) {
    if let Ok(line) = serde_json::to_string(value) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

fn print_disclosure(disclosure: &ai::text::conversation::ConversationDisclosure, accepted: bool) {
    let mut stderr = std::io::stderr().lock();
    let kinds: Vec<Value> = disclosure.kinds.iter().map(|kind| json!(kind)).collect();
    let _ = writeln!(
        stderr,
        "agent consent: {} {} sends the prompt and {} with a spend limit of USD {:.2}",
        disclosure.provider.as_str(),
        disclosure.model.as_str(),
        Value::Array(kinds),
        disclosure.limit.micros() as f64 / 1_000_000.0,
    );
    for (name, url) in &disclosure.links {
        let _ = writeln!(stderr, "{name}: {}", url.as_str());
    }
    if !accepted {
        let _ = writeln!(
            stderr,
            "consent: refused; pass --yes-upload to authorize this exact scope"
        );
    }
}

fn ai_error(error: ai::Error) -> CliError {
    CliError::Engine(EngineError::Ai(error))
}

fn agent_error(error: agent::Error) -> CliError {
    match error {
        agent::Error::Session(error) => CliError::Engine(error.into()),
        agent::Error::Project(error) => CliError::Project(error),
        agent::Error::Chat(error) => ai_error(error),
        agent::Error::Limits => CliError::Usage("--max-steps must be 1..=64".into()),
    }
}

fn usage() -> CliError {
    CliError::Usage(usage_text().into())
}
fn usage_text() -> &'static str {
    "usage: vibecut-cli agent <project> \"<prompt>\" [--apply] [--provider anthropic|openai] \
     [--model <id>] [--max-spend <usd>] [--max-steps N] [--share-clip-names] \
     [--share-transcript] --yes-upload"
}
fn usage_output() -> Output {
    Output::new(
        json!({"usage": usage_text()}),
        usage_text().to_owned(),
        false,
        0,
    )
}
