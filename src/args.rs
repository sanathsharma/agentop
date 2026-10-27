use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

pub fn get_matches() -> ArgMatches {
    Command::new("agentop")
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "Dashboard for AI coding agents running across every session of your tmux server. \
       Select one and press Enter to jump to it.",
        )
        .arg(
            Arg::new("interval")
                .short('i')
                .long("interval")
                .help("Refresh interval in milliseconds")
                .value_name("MS")
                .default_value("1000")
                .value_parser(value_parser!(u64))
                .global(true),
        )
        .arg(
            Arg::new("no_preview")
                .long("no-preview")
                .help("Start with the pane preview hidden")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("stay")
                .short('s')
                .long("stay")
                .help("Keep the dashboard open after switching (useful in a dedicated window)")
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("list")
                .about("Print agents once and exit (for scripts and status bars)")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Output JSON instead of a table")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(Command::new("next").about("Jump straight to the next agent that needs input"))
        .subcommand(
            Command::new("hook")
                .about("Claude Code hook handler. Reads hook JSON on stdin; not for direct use")
                .arg(
                    Arg::new("notify")
                        .long("notify")
                        .help("Send a desktop notification: `input` (needs input only) or `all` (also when a turn finishes)")
                        .value_name("MODE")
                        .num_args(0..=1)
                        .default_missing_value("all")
                        .value_parser(["input", "all"]),
                )
                .arg(
                    Arg::new("sound")
                        .long("sound")
                        .help("Play a sound with each notification")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("init").about("Print the tmux bindings and agent hook configs to install"),
        )
        .get_matches()
}
