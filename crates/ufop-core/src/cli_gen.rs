//! Shell completions and man pages generated from a `clap::Command` tree.
//!
//! `clap_complete` / `clap_mangen` are not available to this build, so the
//! generators here walk the built command tree themselves (subcommands and
//! their visible aliases, long/short options, whether an option takes a
//! value, its possible values and path hints) and print scripts for bash,
//! zsh, fish, PowerShell and Elvish, and roff man pages. `ufop completions`,
//! `ufop man`, `ufop-server completions` and `ufop-server man` use them, and
//! the Linux and Homebrew packages install their output.

use clap::{Arg, ArgAction, Command, ValueHint};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Shells with completion scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Elvish,
}

impl Shell {
    pub const ALL: [Shell; 5] = [Self::Bash, Self::Zsh, Self::Fish, Self::PowerShell, Self::Elvish];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "fish" => Self::Fish,
            "powershell" | "pwsh" => Self::PowerShell,
            "elvish" => Self::Elvish,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
            Self::Elvish => "elvish",
        }
    }

    /// Conventional file name of the script for `bin` (as installed by
    /// packages: bash-completion, zsh `site-functions`, fish
    /// `vendor_completions.d`).
    pub fn file_name(self, bin: &str) -> String {
        match self {
            Self::Bash => bin.to_string(),
            Self::Zsh => format!("_{bin}"),
            Self::Fish => format!("{bin}.fish"),
            Self::PowerShell => format!("_{bin}.ps1"),
            Self::Elvish => format!("{bin}.elv"),
        }
    }
}

/// How an option's value is completed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Values {
    None,
    Any,
    File,
    Dir,
    Choices(Vec<String>),
}

#[derive(Debug, Clone)]
struct Opt {
    longs: Vec<String>,
    shorts: Vec<char>,
    help: String,
    values: Values,
    repeatable: bool,
    /// Declared `global` on a parent: listed once at the top, not per command.
    global: bool,
}

#[derive(Debug, Clone)]
struct Node {
    /// Names from the binary down (`["ufop", "backup", "run"]`).
    path: Vec<String>,
    about: String,
    long_about: String,
    /// (name, about) of each visible subcommand, then its visible aliases.
    subs: Vec<(String, String)>,
    opts: Vec<Opt>,
    /// Positional arguments: (value name, help, values, required, multiple).
    positionals: Vec<(String, String, Values, bool, bool)>,
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn values_of(a: &Arg) -> Values {
    if !a.get_action().takes_values() {
        return Values::None;
    }
    let pv: Vec<String> = a.get_possible_values().iter().filter(|v| !v.is_hide_set()).map(|v| v.get_name().to_string()).collect();
    if !pv.is_empty() {
        return Values::Choices(pv);
    }
    match a.get_value_hint() {
        ValueHint::DirPath => Values::Dir,
        ValueHint::FilePath | ValueHint::AnyPath | ValueHint::ExecutablePath => Values::File,
        _ => {
            // Without an explicit hint, path-like names complete files.
            let id = a.get_id().as_str().to_ascii_lowercase();
            let named = |w: &str| id == w || id.ends_with(&format!("_{w}")) || id.starts_with(&format!("{w}_"));
            if ["path", "file", "dir", "folder", "dbpath", "dest", "target", "source", "sample", "out", "tempdir"].iter().any(|w| named(w)) {
                Values::File
            } else {
                Values::Any
            }
        }
    }
}

fn collect(cmd: &Command, path: Vec<String>, out: &mut Vec<Node>) {
    let mut node = Node {
        about: cmd.get_about().map(|s| first_line(&s.to_string())).unwrap_or_default(),
        long_about: cmd.get_long_about().or(cmd.get_about()).map(|s| s.to_string()).unwrap_or_default(),
        path: path.clone(),
        subs: Vec::new(),
        opts: Vec::new(),
        positionals: Vec::new(),
    };
    for a in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        let help = a.get_help().map(|h| first_line(&h.to_string())).unwrap_or_default();
        if a.is_positional() {
            let name = a.get_value_names().and_then(|n| n.first()).map(|n| n.to_string()).unwrap_or_else(|| a.get_id().to_string().to_uppercase());
            let multiple = a.get_num_args().is_some_and(|n| n.max_values() > 1) || matches!(a.get_action(), ArgAction::Append);
            node.positionals.push((name, help, values_of(a), a.is_required_set(), multiple));
            continue;
        }
        let mut longs: Vec<String> = a.get_long().map(String::from).into_iter().collect();
        longs.extend(a.get_visible_aliases().unwrap_or_default().into_iter().map(String::from));
        let mut shorts: Vec<char> = a.get_short().into_iter().collect();
        shorts.extend(a.get_visible_short_aliases().unwrap_or_default());
        if longs.is_empty() && shorts.is_empty() {
            continue;
        }
        let repeatable = matches!(a.get_action(), ArgAction::Count | ArgAction::Append);
        node.opts.push(Opt { longs, shorts, help, values: values_of(a), repeatable, global: a.is_global_set() });
    }
    let subs: Vec<&Command> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();
    for s in &subs {
        let about = s.get_about().map(|a| first_line(&a.to_string())).unwrap_or_default();
        node.subs.push((s.get_name().to_string(), about.clone()));
        for alias in s.get_visible_aliases() {
            node.subs.push((alias.to_string(), about.clone()));
        }
    }
    out.push(node);
    for s in subs {
        // The generated `help` subcommand mirrors the tree; its own
        // children add nothing to complete.
        if s.get_name() == "help" {
            continue;
        }
        let mut p = path.clone();
        p.push(s.get_name().to_string());
        collect(s, p, out);
    }
}

fn nodes(cmd: &Command) -> Vec<Node> {
    let mut c = cmd.clone();
    c.build();
    let mut out = Vec::new();
    collect(&c, vec![c.get_name().to_string()], &mut out);
    out
}

// ── Command help for the desktop terminal's help drawer ────────────────

/// One command of a CLI as the terminal's help drawer lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandHelp {
    /// The words after the program: `["backup", "run"]`.
    pub path: Vec<String>,
    /// One line on what the command does.
    pub about: String,
    /// The words to type, with its arguments as placeholders:
    /// `backup run <JOB> [OPTIONS]`.
    pub usage: String,
    /// Has subcommands of its own, so typing it alone only starts a command.
    pub has_subcommands: bool,
    pub options: Vec<OptionHelp>,
}

/// One option of a [`CommandHelp`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionHelp {
    /// `--dry-run`, `-v`…
    pub flags: Vec<String>,
    pub help: String,
    /// Takes a value (`--limit 1M`) rather than being a switch.
    pub takes_value: bool,
}

/// Every visible command below the root of `cmd` (not the root itself),
/// parents before their subcommands, in the order the CLI lists them.
pub fn describe(cmd: &Command) -> Vec<CommandHelp> {
    nodes(cmd)
        .into_iter()
        .filter(|n| n.path.len() > 1)
        .map(|n| {
            let words: Vec<String> = n.path[1..].to_vec();
            let mut usage = words.join(" ");
            for (name, _, _, required, multiple) in &n.positionals {
                let dots = if *multiple { "..." } else { "" };
                let _ = if *required { write!(usage, " <{name}>{dots}") } else { write!(usage, " [{name}]{dots}") };
            }
            if !n.subs.is_empty() {
                usage.push_str(" <COMMAND>");
            }
            if !n.opts.is_empty() {
                usage.push_str(" [OPTIONS]");
            }
            CommandHelp {
                path: words,
                about: n.about,
                usage,
                has_subcommands: !n.subs.is_empty(),
                // Global options (--json, --profile…) work everywhere; listing them
                // under every command would bury the command's own.
                options: n
                    .opts
                    .iter()
                    .filter(|o| !o.global || n.path.len() == 1)
                    .map(|o| OptionHelp { flags: opt_words(o), help: o.help.clone(), takes_value: o.values != Values::None })
                    .collect(),
            }
        })
        .collect()
}

/// `describe` as the JSON `ufop help-json` prints and the bundled snapshot holds.
pub fn describe_json(cmd: &Command) -> String {
    serde_json::to_string(&describe(cmd)).unwrap_or_else(|_| "[]".into())
}

/// The `ufop` commands as of this build, from the snapshot the `ufop` CLI's
/// own tests keep current (`scripts/gen-cli-help.sh` rewrites it). The
/// desktop app lists these without needing the CLI installed.
pub fn bundled_ufop_help() -> Vec<CommandHelp> {
    serde_json::from_str(include_str!("../data/ufop-cli-help.json")).unwrap_or_default()
}

/// Identifier-safe form of a binary name (`ufop-server` → `ufop_server`).
fn ident(bin: &str) -> String {
    bin.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// The completion script for `shell`.
pub fn completions(cmd: &Command, shell: Shell) -> String {
    let nodes = nodes(cmd);
    let bin = cmd.get_bin_name().unwrap_or(cmd.get_name()).to_string();
    match shell {
        Shell::Bash => bash(&bin, &nodes),
        Shell::Zsh => zsh(&bin, &nodes),
        Shell::Fish => fish(&bin, &nodes),
        Shell::PowerShell => powershell(&bin, &nodes),
        Shell::Elvish => elvish(&bin, &nodes),
    }
}

fn opt_words(o: &Opt) -> Vec<String> {
    o.longs.iter().map(|l| format!("--{l}")).chain(o.shorts.iter().map(|s| format!("-{s}"))).collect()
}

// ── bash ────────────────────────────────────────────────────────────────

fn bash(bin: &str, nodes: &[Node]) -> String {
    let f = format!("_{}", ident(bin));
    let key = |n: &Node| n.path.join("__");
    let mut s = String::new();
    let _ = writeln!(s, "# bash completion for {bin} (generated by `{bin} completions bash`)");
    let _ = writeln!(s, "\n{f}_subs() {{\n    case \"$1\" in");
    for n in nodes.iter().filter(|n| !n.subs.is_empty()) {
        let names: Vec<&str> = n.subs.iter().map(|(n, _)| n.as_str()).collect();
        let _ = writeln!(s, "        '{}') echo '{}' ;;", key(n), names.join(" "));
    }
    let _ = writeln!(s, "    esac\n}}\n\n# Possible values of positional arguments.\n{f}_pos() {{\n    case \"$1\" in");
    for n in nodes {
        let vals: Vec<&str> = n.positionals.iter().filter_map(|(_, _, v, _, _)| if let Values::Choices(c) = v { Some(c) } else { None }).flatten().map(|v| v.as_str()).collect();
        if !vals.is_empty() {
            let _ = writeln!(s, "        '{}') echo '{}' ;;", key(n), vals.join(" "));
        }
    }
    let _ = writeln!(s, "    esac\n}}\n\n{f}_opts() {{\n    case \"$1\" in");
    for n in nodes {
        let words: Vec<String> = n.opts.iter().flat_map(opt_words).collect();
        let _ = writeln!(s, "        '{}') echo '{}' ;;", key(n), words.join(" "));
    }
    let _ = writeln!(s, "    esac\n}}\n\n{f}_valopts() {{\n    case \"$1\" in");
    for n in nodes {
        let words: Vec<String> = n.opts.iter().filter(|o| o.values != Values::None).flat_map(opt_words).collect();
        if !words.is_empty() {
            let _ = writeln!(s, "        '{}') echo '{}' ;;", key(n), words.join(" "));
        }
    }
    let _ = writeln!(s, "    esac\n}}\n\n# Prints @file, @dir, @any or the possible values of option $2 of command $1.\n{f}_value() {{\n    case \"$1|$2\" in");
    for n in nodes {
        for o in n.opts.iter().filter(|o| o.values != Values::None) {
            let v = match &o.values {
                Values::Choices(c) => c.join(" "),
                Values::Dir => "@dir".into(),
                Values::File => "@file".into(),
                _ => "@any".into(),
            };
            let pats: Vec<String> = opt_words(o).iter().map(|w| format!("'{}|{w}'", key(n))).collect();
            let _ = writeln!(s, "        {}) echo '{}' ;;", pats.join(" | "), v.replace('\'', ""));
        }
    }
    let _ = write!(
        s,
        r#"    esac
}}

{f}_reply() {{
    # $1: compgen mode (-W words, -f, -d), $2: words, $3: current word
    COMPREPLY=()
    local line
    while IFS= read -r line; do
        COMPREPLY+=("$line")
    done < <(if [[ "$1" == "-W" ]]; then compgen -W "$2" -- "$3"; else compgen "$1" -- "$3"; fi)
}}

{f}() {{
    local cur prev path i w skip v
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev=""
    if (( COMP_CWORD > 0 )); then prev="${{COMP_WORDS[COMP_CWORD-1]}}"; fi
    path="{root}"
    skip=0
    for (( i = 1; i < COMP_CWORD; i++ )); do
        w="${{COMP_WORDS[i]}}"
        if (( skip )); then skip=0; continue; fi
        case "$w" in
            -*=*) continue ;;
            -*)
                case " $({f}_valopts "$path") " in *" $w "*) skip=1 ;; esac
                continue ;;
        esac
        case " $({f}_subs "$path") " in *" $w "*) path="${{path}}__${{w}}" ;; esac
    done
    if [[ "$prev" == -* ]]; then
        case " $({f}_valopts "$path") " in
            *" $prev "*)
                v="$({f}_value "$path" "$prev")"
                case "$v" in
                    @dir) {f}_reply -d "" "$cur" ;;
                    @file) {f}_reply -f "" "$cur" ;;
                    @any) COMPREPLY=() ;;
                    *) {f}_reply -W "$v" "$cur" ;;
                esac
                return 0 ;;
        esac
    fi
    if [[ "$cur" == -* ]]; then
        {f}_reply -W "$({f}_opts "$path")" "$cur"
    else
        {f}_reply -W "$({f}_subs "$path") $({f}_pos "$path")" "$cur"
        if (( ${{#COMPREPLY[@]}} == 0 )); then {f}_reply -f "" "$cur"; fi
    fi
    return 0
}}

complete -F {f} -o bashdefault -o default {bin}
"#,
        root = bin
    );
    s
}

// ── zsh ─────────────────────────────────────────────────────────────────

fn zsh_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "'\\''").replace('[', "\\[").replace(']', "\\]").replace(':', "\\:")
}

/// Completion action for a value (`_files`, `(a b)`; empty = none).
fn zsh_value_action(v: &Values) -> String {
    match v {
        Values::None | Values::Any => String::new(),
        Values::File => "_files".into(),
        Values::Dir => "_files -/".into(),
        Values::Choices(c) => format!("({})", c.iter().map(|x| zsh_escape(x)).collect::<Vec<_>>().join(" ")),
    }
}

/// The `: :action` suffix of an option spec (empty for flags).
fn zsh_action(v: &Values) -> String {
    if *v == Values::None {
        String::new()
    } else {
        format!(": :{}", zsh_value_action(v))
    }
}

fn zsh(bin: &str, nodes: &[Node]) -> String {
    let fname = |n: &Node| format!("_{}", n.path.iter().map(|p| ident(p)).collect::<Vec<_>>().join("__"));
    let mut s = format!("#compdef {bin}\n# zsh completion for {bin} (generated by `{bin} completions zsh`)\n");
    for n in nodes {
        let _ = writeln!(s, "\n{}() {{", fname(n));
        let _ = writeln!(s, "    local curcontext=\"$curcontext\" state line ret=1\n    typeset -A opt_args");
        let mut specs: Vec<String> = Vec::new();
        for o in &n.opts {
            let words = opt_words(o);
            let excl = if o.repeatable || words.len() < 2 { String::new() } else { format!("({})", words.join(" ")) };
            let rep = if o.repeatable { "*" } else { "" };
            for l in &o.longs {
                let eq = if o.values == Values::None { "" } else { "=" };
                specs.push(format!("'{excl}{rep}--{l}{eq}[{}]{}'", zsh_escape(&o.help), zsh_action(&o.values)));
            }
            for c in &o.shorts {
                let plus = if o.values == Values::None { "" } else { "+" };
                specs.push(format!("'{excl}{rep}-{c}{plus}[{}]{}'", zsh_escape(&o.help), zsh_action(&o.values)));
            }
        }
        if n.subs.is_empty() {
            for (name, help, v, _, multiple) in &n.positionals {
                let act = match zsh_value_action(v) {
                    a if a.is_empty() => "_default".to_string(),
                    a => a,
                };
                let label = if help.is_empty() { name.clone() } else { help.clone() };
                specs.push(format!("'{}:{}:{act}'", if *multiple { "*" } else { "" }, zsh_escape(&label)));
            }
            let _ = writeln!(s, "    _arguments -s -S \\\n        {} && ret=0\n    return ret\n}}", if specs.is_empty() { "'*: :_default'".to_string() } else { specs.join(" \\\n        ") });
            continue;
        }
        specs.push("'1: :->cmd'".into());
        specs.push("'*:: :->args'".into());
        let _ = writeln!(s, "    _arguments -C -s -S \\\n        {} && ret=0", specs.join(" \\\n        "));
        let _ = writeln!(s, "    case $state in\n        cmd)\n            local -a commands\n            commands=(");
        for (name, about) in &n.subs {
            let _ = writeln!(s, "                '{}:{}'", zsh_escape(name), zsh_escape(about));
        }
        let _ = writeln!(s, "            )\n            _describe -t commands '{} command' commands && ret=0\n            ;;\n        args)\n            case $line[1] in", n.path.join(" "));
        for (name, _) in &n.subs {
            let target = nodes.iter().find(|m| m.path.len() == n.path.len() + 1 && m.path[..n.path.len()] == n.path[..] && (m.path.last() == Some(name) || m_alias(n, m, name)));
            if let Some(m) = target {
                let _ = writeln!(s, "                '{}') {} && ret=0 ;;", zsh_escape(name), fname(m));
            }
        }
        let _ = writeln!(s, "            esac\n            ;;\n    esac\n    return ret\n}}");
    }
    let root = &nodes[0];
    let _ = writeln!(s, "\nif [ \"$funcstack[1]\" = \"_{}\" ]; then\n    {} \"$@\"\nelse\n    compdef {} {bin}\nfi", ident(bin), fname(root), fname(root));
    s
}

/// `name` is an alias listed in `parent.subs` right after `m`'s own name.
fn m_alias(parent: &Node, m: &Node, name: &str) -> bool {
    let Some(own) = m.path.last() else { return false };
    let Some(i) = parent.subs.iter().position(|(n, _)| n == own) else { return false };
    parent.subs[i + 1..].iter().take_while(|(n, a)| *a == parent.subs[i].1 && n != own).any(|(n, _)| n == name)
}

// ── fish ────────────────────────────────────────────────────────────────

fn fish_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn fish(bin: &str, nodes: &[Node]) -> String {
    let id = ident(bin);
    let mut s = format!("# fish completion for {bin} (generated by `{bin} completions fish`)\n\nfunction __fish_{id}_subs\n    switch \"$argv\"\n");
    for n in nodes.iter().filter(|n| !n.subs.is_empty()) {
        let names: Vec<&str> = n.subs.iter().map(|(n, _)| n.as_str()).collect();
        let _ = writeln!(s, "        case '{}'\n            printf '%s\\n' {}", fish_escape(&n.path.join(" ")), names.iter().map(|x| format!("'{}'", fish_escape(x))).collect::<Vec<_>>().join(" "));
    }
    let _ = writeln!(
        s,
        "    end\nend\n\n# The subcommand path typed so far (`{bin} backup run`).\nfunction __fish_{id}_current\n    set -l path {bin}\n    for t in (commandline -opc)[2..-1]\n        if string match -q -- '-*' $t\n            continue\n        end\n        if contains -- $t (__fish_{id}_subs $path)\n            set path \"$path $t\"\n        end\n    end\n    echo $path\nend\n"
    );
    for n in nodes {
        let cond = format!("-n 'test (__fish_{id}_current) = \"{}\"'", fish_escape(&n.path.join(" ")));
        for (name, about) in &n.subs {
            let _ = writeln!(s, "complete -c {bin} {cond} -f -a '{}' -d '{}'", fish_escape(name), fish_escape(about));
        }
        for o in &n.opts {
            let mut line = format!("complete -c {bin} {cond}");
            for l in &o.longs {
                let _ = write!(line, " -l '{}'", fish_escape(l));
            }
            for c in &o.shorts {
                let _ = write!(line, " -s '{c}'");
            }
            match &o.values {
                Values::None => {}
                Values::Any => line.push_str(" -r -f"),
                Values::File => line.push_str(" -r -F"),
                Values::Dir => line.push_str(" -r -f -a '(__fish_complete_directories)'"),
                Values::Choices(c) => {
                    let _ = write!(line, " -r -f -a '{}'", fish_escape(&c.join(" ")));
                }
            }
            if !o.help.is_empty() {
                let _ = write!(line, " -d '{}'", fish_escape(&o.help));
            }
            s.push_str(&line);
            s.push('\n');
        }
    }
    s
}

// ── PowerShell ──────────────────────────────────────────────────────────

fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn powershell(bin: &str, nodes: &[Node]) -> String {
    let key = |n: &Node| n.path.join(";");
    let mut s = format!(
        "# PowerShell completion for {bin} (generated by `{bin} completions powershell`)\n# Load it from your profile: {bin} completions powershell | Out-String | Invoke-Expression\n\nusing namespace System.Management.Automation\nusing namespace System.Management.Automation.Language\n\nRegister-ArgumentCompleter -Native -CommandName '{bin}' -ScriptBlock {{\n    param($wordToComplete, $commandAst, $cursorPosition)\n\n    $subs = @{{\n"
    );
    for n in nodes.iter().filter(|n| !n.subs.is_empty()) {
        let names: Vec<String> = n.subs.iter().map(|(n, _)| format!("'{}'", ps_escape(n))).collect();
        let _ = writeln!(s, "        '{}' = @({})", ps_escape(&key(n)), names.join(", "));
    }
    let _ = writeln!(s, "    }}\n    $values = @{{");
    for n in nodes {
        for o in &n.opts {
            if let Values::Choices(c) = &o.values {
                let list: Vec<String> = c.iter().map(|v| format!("'{}'", ps_escape(v))).collect();
                for w in opt_words(o) {
                    let _ = writeln!(s, "        '{}|{}' = @({})", ps_escape(&key(n)), ps_escape(&w), list.join(", "));
                }
            }
        }
    }
    let _ = write!(
        s,
        r#"    }}
    $command = '{bin}'
    $prev = ''
    $elements = $commandAst.CommandElements
    for ($i = 1; $i -lt $elements.Count; $i++) {{
        $e = $elements[$i]
        if ($e.Extent.EndOffset -ge $cursorPosition) {{ break }}
        $prev = "$e"
        if ($e -isnot [StringConstantExpressionAst] -or $e.StringConstantType -ne [StringConstantType]::BareWord) {{ continue }}
        if ($e.Value.StartsWith('-')) {{ continue }}
        if ($subs.ContainsKey($command) -and $subs[$command] -contains $e.Value) {{ $command = "$command;$($e.Value)" }}
    }}
    if ($values.ContainsKey("$command|$prev")) {{
        return $values["$command|$prev"] | Where-Object {{ $_ -like "$wordToComplete*" }} |
            ForEach-Object {{ [CompletionResult]::new($_, $_, [CompletionResultType]::ParameterValue, $_) }}
    }}

    $completions = @(switch ($command) {{
"#
    );
    for n in nodes {
        let _ = writeln!(s, "        '{}' {{", ps_escape(&key(n)));
        for (name, about) in &n.subs {
            let tip = if about.is_empty() { name.clone() } else { about.clone() };
            let _ = writeln!(s, "            [CompletionResult]::new('{}', '{}', [CompletionResultType]::ParameterValue, '{}')", ps_escape(name), ps_escape(name), ps_escape(&tip));
        }
        for o in &n.opts {
            let tip = if o.help.is_empty() { opt_words(o).join(", ") } else { o.help.clone() };
            for w in opt_words(o) {
                let _ = writeln!(s, "            [CompletionResult]::new('{}', '{}', [CompletionResultType]::ParameterName, '{}')", ps_escape(&w), ps_escape(&w), ps_escape(&tip));
            }
        }
        let _ = writeln!(s, "            break\n        }}");
    }
    let _ = write!(
        s,
        r#"    }})

    $completions.Where{{ $_.CompletionText -like "$wordToComplete*" }} |
        Sort-Object -Property ListItemText
}}
"#
    );
    s
}

// ── Elvish ──────────────────────────────────────────────────────────────

fn elvish_escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn elvish(bin: &str, nodes: &[Node]) -> String {
    let key = |n: &Node| n.path.join(";");
    let mut s = format!("# Elvish completion for {bin} (generated by `{bin} completions elvish`)\n\nuse builtin\nuse str\n\nset edit:completion:arg-completer[{bin}] = {{|@words|\n    fn cand {{|text desc|\n        edit:complex-candidate $text &display=$text' '$desc\n    }}\n    var subs = [\n");
    for n in nodes {
        let names: Vec<String> = n.subs.iter().map(|(n, _)| format!("'{}'", elvish_escape(n))).collect();
        let _ = writeln!(s, "        &'{}'= [{}]", elvish_escape(&key(n)), names.join(" "));
    }
    let _ = writeln!(s, "    ]\n    var values = [");
    for n in nodes {
        for o in &n.opts {
            if let Values::Choices(c) = &o.values {
                let list: Vec<String> = c.iter().map(|v| format!("'{}'", elvish_escape(v))).collect();
                for w in opt_words(o) {
                    let _ = writeln!(s, "        &'{}|{}'= [{}]", elvish_escape(&key(n)), elvish_escape(&w), list.join(" "));
                }
            }
        }
    }
    let _ = write!(
        s,
        r#"    ]
    var command = '{bin}'
    for word $words[1..-1] {{
        if (str:has-prefix $word '-') {{
            continue
        }}
        if (and (has-key $subs $command) (has-value $subs[$command] $word)) {{
            set command = $command';'$word
        }}
    }}
    var completions = [
"#
    );
    for n in nodes {
        let _ = writeln!(s, "        &'{}'= {{", elvish_escape(&key(n)));
        for (name, about) in &n.subs {
            let _ = writeln!(s, "            cand '{}' '{}'", elvish_escape(name), elvish_escape(about));
        }
        for o in &n.opts {
            for w in opt_words(o) {
                let _ = writeln!(s, "            cand '{}' '{}'", elvish_escape(&w), elvish_escape(&o.help));
            }
        }
        let _ = writeln!(s, "        }}");
    }
    let _ = write!(
        s,
        r#"    ]
    var prev = ''
    if (> (count $words) 2) {{
        set prev = $words[-2]
    }}
    if (has-key $values $command'|'$prev) {{
        for v $values[$command'|'$prev] {{
            put $v
        }}
    }} elif (has-key $completions $command) {{
        $completions[$command]
    }}
}}
"#
    );
    s
}

// ── man pages ───────────────────────────────────────────────────────────

fn roff(s: &str) -> String {
    let e = s.replace('\\', "\\e").replace('-', "\\-");
    e.lines().map(|l| if l.starts_with('.') || l.starts_with('\'') { format!("\\&{l}") } else { l.to_string() }).collect::<Vec<_>>().join("\n")
}

/// Man pages for `cmd` and every subcommand: (file name, roff source),
/// e.g. `ufop.1`, `ufop-backup.1`, `ufop-backup-run.1`.
pub fn man_pages(cmd: &Command) -> Vec<(String, String)> {
    man_pages_in(cmd, "1")
}

/// [`man_pages`] in manual `section` (`ufop-server` uses 8, system
/// daemons, so its pages never collide with `ufop server …` in section 1).
pub fn man_pages_in(cmd: &Command, section: &str) -> Vec<(String, String)> {
    let version = cmd.get_version().unwrap_or("").to_string();
    let nodes = nodes(cmd);
    nodes
        .iter()
        .map(|n| {
            let page = n.path.join("-");
            let mut s = String::new();
            let title = if section == "8" { "System Administration" } else { "User Commands" };
            let _ = writeln!(s, ".TH \"{}\" \"{section}\" \"\" \"{} {}\" \"{title}\"", page.to_uppercase(), roff(&n.path[0]), roff(&version));
            let _ = writeln!(s, ".SH NAME\n{} \\- {}", roff(&page), roff(if n.about.is_empty() { &page } else { &n.about }));
            let mut syn = format!("\\fB{}\\fR", roff(&n.path.join(" ")));
            if !n.opts.is_empty() {
                syn.push_str(" [\\fIOPTIONS\\fR]");
            }
            if !n.subs.is_empty() {
                syn.push_str(" <\\fICOMMAND\\fR>");
            }
            for (name, _, _, required, multiple) in &n.positionals {
                let dots = if *multiple { "..." } else { "" };
                syn.push_str(&if *required { format!(" <\\fI{}\\fR>{dots}", roff(name)) } else { format!(" [\\fI{}\\fR]{dots}", roff(name)) });
            }
            let _ = writeln!(s, ".SH SYNOPSIS\n{syn}");
            if !n.long_about.is_empty() {
                let _ = writeln!(s, ".SH DESCRIPTION\n{}", roff(&n.long_about).replace("\n\n", "\n.PP\n"));
            }
            if !n.opts.is_empty() || !n.positionals.is_empty() {
                let _ = writeln!(s, ".SH OPTIONS");
                for o in &n.opts {
                    let names: Vec<String> = o.shorts.iter().map(|c| format!("\\fB\\-{c}\\fR")).chain(o.longs.iter().map(|l| format!("\\fB\\-\\-{}\\fR", roff(l)))).collect();
                    let val = if o.values == Values::None { String::new() } else { " \\fIVALUE\\fR".into() };
                    let _ = writeln!(s, ".TP\n{}{val}", names.join(", "));
                    let mut help = roff(&o.help);
                    if let Values::Choices(c) = &o.values {
                        let _ = write!(help, " [possible values: {}]", roff(&c.join(", ")));
                    }
                    let _ = writeln!(s, "{help}");
                }
                for (name, help, _, _, _) in &n.positionals {
                    let _ = writeln!(s, ".TP\n<\\fI{}\\fR>\n{}", roff(name), roff(help));
                }
            }
            if !n.subs.is_empty() {
                let _ = writeln!(s, ".SH SUBCOMMANDS");
                for (name, about) in &n.subs {
                    let child = format!("{page}-{name}");
                    let _ = writeln!(s, ".TP\n\\fB{}({section})\\fR\n{}", roff(&child), roff(about));
                }
            }
            if n.path.len() > 1 {
                let _ = writeln!(s, ".SH SEE ALSO\n\\fB{}({section})\\fR", roff(&n.path[..n.path.len() - 1].join("-")));
            }
            if !version.is_empty() {
                let _ = writeln!(s, ".SH VERSION\nv{}", roff(&version));
            }
            (format!("{page}.{section}"), s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Parser, Subcommand};

    #[derive(Parser)]
    #[command(name = "demo", version = "1.2.3", about = "Demo tool")]
    struct Cli {
        /// Output as JSON
        #[arg(long, global = true)]
        json: bool,
        /// Verbosity
        #[arg(short, long, action = ArgAction::Count, global = true)]
        verbose: u8,
        #[command(subcommand)]
        cmd: Cmd,
    }

    #[derive(Subcommand)]
    enum Cmd {
        /// Back things up
        #[command(subcommand, visible_alias = "bk")]
        Backup(BackupCmd),
        /// Print the name's [section]: it's here
        Status {
            #[arg(long, value_parser = ["short", "long"])]
            style: Option<String>,
            paths: Vec<String>,
        },
        #[command(hide = true)]
        Secret,
    }

    #[derive(Subcommand)]
    enum BackupCmd {
        /// Run a backup now
        Run {
            /// Destination folder
            #[arg(long, value_hint = ValueHint::DirPath)]
            dest: String,
            #[arg(long)]
            dbpath: Option<String>,
            #[arg(long, value_parser = ["fixed", "fastcdc"])]
            chunker: Option<String>,
        },
        /// List versions
        List,
    }

    fn cmd() -> Command {
        use clap::CommandFactory;
        Cli::command()
    }

    #[test]
    fn every_shell_lists_subcommands_options_and_values() {
        for sh in Shell::ALL {
            let s = completions(&cmd(), sh);
            for w in ["backup", "run", "list", "status", "dest", "chunker", "fastcdc", "json"] {
                assert!(s.contains(w), "{sh:?} lacks {w}:\n{s}");
            }
            assert!(!s.contains("secret"), "{sh:?} shows a hidden command");
        }
    }

    #[test]
    fn bash_script_routes_nested_commands_and_value_options() {
        let s = completions(&cmd(), Shell::Bash);
        assert!(s.contains("'demo') echo 'backup bk status help'"), "{s}");
        assert!(s.contains("'demo__backup') echo 'run list help'"));
        assert!(s.contains("'demo__backup__run|--dest') echo '@dir'"));
        assert!(s.contains("'demo__backup__run|--dbpath') echo '@file'"));
        assert!(s.contains("'demo__backup__run|--chunker') echo 'fixed fastcdc'"));
        // Global options reach subcommands.
        assert!(s.lines().any(|l| l.contains("'demo__backup__run')") && l.contains("--json") && l.contains("-v")));
        assert!(s.contains("complete -F _demo -o bashdefault -o default demo"));
    }

    #[test]
    fn bash_script_is_valid_bash() {
        let Ok(out) = std::process::Command::new("bash").arg("-n").arg("-c").arg(completions(&cmd(), Shell::Bash)).output() else { return };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn zsh_fish_powershell_elvish_shapes() {
        let z = completions(&cmd(), Shell::Zsh);
        assert!(z.starts_with("#compdef demo"));
        assert!(z.contains("_demo__backup__run() {"));
        assert!(z.contains("'--dest=[Destination folder]: :_files -/'"), "{z}");
        assert!(z.contains("'bk') _demo__backup && ret=0 ;;"), "{z}");
        assert!(z.contains("\\[section\\]\\: it'\\''s here"), "{z}");
        let f = completions(&cmd(), Shell::Fish);
        assert!(f.contains("complete -c demo -n 'test (__fish_demo_current) = \"demo backup run\"' -l 'chunker' -r -f -a 'fixed fastcdc'"), "{f}");
        assert!(f.contains("-a '(__fish_complete_directories)'"));
        let p = completions(&cmd(), Shell::PowerShell);
        assert!(p.contains("Register-ArgumentCompleter -Native -CommandName 'demo'"));
        assert!(p.contains("'demo;backup' = @('run', 'list', 'help')"), "{p}");
        let e = completions(&cmd(), Shell::Elvish);
        assert!(e.contains("set edit:completion:arg-completer[demo]"));
        assert!(e.contains("&'demo;backup;run'= {"));
    }

    #[test]
    fn man_pages_cover_the_tree() {
        let pages = man_pages(&cmd());
        let names: Vec<&str> = pages.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"demo.1") && names.contains(&"demo-backup.1") && names.contains(&"demo-backup-run.1"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("secret")));
        let run = &pages.iter().find(|(n, _)| n == "demo-backup-run.1").unwrap().1;
        assert!(run.starts_with(".TH \"DEMO-BACKUP-RUN\" \"1\""));
        assert!(run.contains("\\fB\\-\\-dest\\fR \\fIVALUE\\fR"));
        assert!(run.contains("[possible values: fixed, fastcdc]"));
        assert!(run.contains(".SH SEE ALSO\n\\fBdemo\\-backup(1)\\fR"));
        let top = &pages[0].1;
        assert!(top.contains("\\fBdemo\\-backup(1)\\fR") && top.contains("v1.2.3"));
        let eight = man_pages_in(&cmd(), "8");
        assert_eq!(eight[0].0, "demo.8");
        assert!(eight[0].1.starts_with(".TH \"DEMO\" \"8\"") && eight[0].1.contains("\\fBdemo\\-backup(8)\\fR"));
        assert_eq!(roff(".x - \\y"), "\\&.x \\- \\ey");
    }

    #[test]
    fn shells_parse_and_name_files() {
        assert_eq!(Shell::parse("PWSH"), Some(Shell::PowerShell));
        assert_eq!(Shell::parse("tcsh"), None);
        assert_eq!(Shell::Zsh.file_name("ufop"), "_ufop");
        assert_eq!(Shell::Fish.file_name("ufop"), "ufop.fish");
        assert_eq!(ident("ufop-server"), "ufop_server");
    }
}
