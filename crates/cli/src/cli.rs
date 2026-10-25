//! Описание командной строки. Грамматика: `discordctl <что> <действие> [цель] [параметры]`.
//!
//! Цель (`TARGET`) везде одна и та же: `@имя` (личный чат или группа), `Сервер/канал` или числовой id.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

const OVERVIEW: &str = "\
\x1b[1mGet started\x1b[0m
  discordctl setup                       install everything and connect an AI agent
  discordctl doctor                      check the installation and tell what to fix

\x1b[1mDiscord\x1b[0m
  discordctl server | channel | chat | friend      list what you have
  discordctl message read|send|edit|delete|react   read and write messages (shortcuts: read, send)
  discordctl voice  join|leave|mute|unmute|deafen|undeafen|status
  discordctl stream list|start|stop|status

\x1b[1mAI agents\x1b[0m
  discordctl mcp install                 connect Claude Code / Claude Desktop / Cursor
  discordctl app add|list|remove|log     rights of apps and agents
  discordctl app approve                 approve what an agent wants to do (keep it open)

\x1b[1mTargets\x1b[0m (the same everywhere)
  @alice    a direct chat or group by name        Server/general   a channel in a server
  123456789012345678   any id                     @*  all chats    Server/*  a whole server

\x1b[1mExamples\x1b[0m
  discordctl read @alice -n 20
  discordctl send \"Server/general\" \"hello\" --dry-run
  discordctl voice join \"Server/Lounge\"
  discordctl mcp install --agent claude-code --preset read-only";

#[derive(Parser)]
#[command(
    name = "discordctl",
    version,
    about = "Control your own Discord client from the terminal and from AI agents",
    long_about = "Control your own Discord client from the terminal and from AI agents.\n\nWorks through the Chromium debugging pipe of the desktop client. Windows only.",
    after_help = OVERVIEW,
    arg_required_else_help = true,
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct Cli {
    /// Machine-readable JSON output
    #[arg(long, global = true)]
    pub json: bool,
    /// Plain output without colors (also honors NO_COLOR)
    #[arg(long, global = true)]
    pub no_color: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Install everything step by step, then offer to connect an AI agent
    Setup(SetupArgs),
    /// Install files and the launch shim (non-interactive); the discord:// link hook is opt-in
    Install {
        /// Discord install folder (default: %LOCALAPPDATA%\Discord)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Also point discord:// links at the launcher (changes a registry value under HKCU)
        #[arg(long)]
        link_hook: bool,
    },
    /// Put Discord back the way it was (a running Discord is not closed)
    Uninstall {
        /// Also delete discordctl data: app tokens, audit log, logs (asks first)
        #[arg(long)]
        purge: bool,
    },
    /// Check the installation and say what to fix
    Doctor {
        /// Repair what is safe to repair: shim, link handler, missing files
        #[arg(long)]
        fix: bool,
    },
    /// What is running: daemon, Discord, hook
    Status,
    /// Start Discord under discordctl
    Start {
        /// Extra arguments for Discord (only a few safe ones are allowed)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Servers you are in
    #[command(visible_alias = "servers")]
    Server {
        #[command(subcommand)]
        cmd: Option<ServerCmd>,
    },
    /// Channels of a server
    #[command(visible_alias = "channels")]
    Channel {
        #[command(subcommand)]
        cmd: ChannelCmd,
    },
    /// Direct chats and group chats
    #[command(visible_alias = "chats")]
    Chat {
        #[command(subcommand)]
        cmd: Option<ChatCmd>,
    },
    /// Friends
    #[command(visible_alias = "friends")]
    Friend {
        #[command(subcommand)]
        cmd: Option<FriendCmd>,
    },
    /// Read, send, edit, delete messages and reactions
    #[command(visible_alias = "msg")]
    Message {
        #[command(subcommand)]
        cmd: MessageCmd,
    },
    /// Shortcut for `message read`
    Read(ReadArgs),
    /// Shortcut for `message send`
    Send(SendArgs),
    /// Voice calls: join, leave, mute, deafen
    Voice {
        #[command(subcommand)]
        cmd: VoiceCmd,
    },
    /// Share a window or screen into the voice channel you are in
    Stream {
        #[command(subcommand)]
        cmd: StreamCmd,
    },

    /// Rights of apps and AI agents (tokens, approvals, audit log)
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Connect AI agents (Claude Code, Claude Desktop, Cursor) through MCP
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// The background daemon that owns the Discord connection
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Shell completions: `discordctl completions powershell | Out-String | Invoke-Expression`
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Tools for working on discordctl itself
    #[command(hide = true)]
    Dev {
        #[command(subcommand)]
        cmd: DevCmd,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Shell {
    Powershell,
    Bash,
    Zsh,
    Fish,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum AgentKind {
    /// Anthropic Claude Code (needs the `claude` command)
    ClaudeCode,
    /// Claude Desktop app
    ClaudeDesktop,
    /// Cursor editor
    Cursor,
    /// Do not touch any app: print the config to paste yourself
    Print,
}

impl AgentKind {
    pub fn slug(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::ClaudeDesktop => "claude-desktop",
            AgentKind::Cursor => "cursor",
            AgentKind::Print => "print",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::ClaudeDesktop => "Claude Desktop",
            AgentKind::Cursor => "Cursor",
            AgentKind::Print => "Other (print the config)",
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// List things and read messages; nothing can be sent
    ReadOnly,
    /// Read, plus send/edit/delete/react: every write waits for your approval
    Assistant,
}

#[derive(Args)]
pub struct SetupArgs {
    /// Answer yes to everything and do not ask (for scripts)
    #[arg(short, long)]
    pub yes: bool,
    /// Connect this agent without asking (repeatable)
    #[arg(long, value_enum)]
    pub agent: Vec<AgentKind>,
    /// Access for the agent
    #[arg(long, value_enum, default_value = "read-only")]
    pub preset: Preset,
    /// Do not offer to restart a running Discord
    #[arg(long)]
    pub no_restart: bool,
    /// Discord install folder (default: %LOCALAPPDATA%\Discord)
    #[arg(long)]
    pub root: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum ServerCmd {
    /// List your servers
    List,
}

#[derive(Subcommand)]
pub enum ChannelCmd {
    /// List the channels of a server
    List {
        /// Server name or id
        server: String,
    },
}

#[derive(Subcommand)]
pub enum ChatCmd {
    /// List direct chats and groups, newest first
    List,
}

#[derive(Subcommand)]
pub enum FriendCmd {
    /// List friends
    List,
}

/// Куда: `@имя`, `Сервер/канал` или id.
const TARGET_HELP: &str = "Chat: @name, Server/channel, or an id";

#[derive(Args)]
pub struct ReadArgs {
    #[arg(help = TARGET_HELP)]
    pub target: String,
    /// How many messages (up to 2000; long reads are paced, so they take a while)
    #[arg(short = 'n', long, default_value_t = 50)]
    pub limit: u32,
    /// Read messages older than this message id
    #[arg(long, conflicts_with = "after")]
    pub before: Option<String>,
    /// Read messages newer than this message id
    #[arg(long)]
    pub after: Option<String>,
}

#[derive(Args)]
pub struct SendArgs {
    #[arg(help = TARGET_HELP)]
    pub target: String,
    /// The message text (words are joined with spaces)
    pub text: Vec<String>,
    /// Reply to the message with this id
    #[arg(long)]
    pub reply_to: Option<String>,
    /// Read the text from stdin
    #[arg(long)]
    pub stdin: bool,
    /// Let @mentions ping people (by default nobody is pinged)
    #[arg(long)]
    pub mentions: bool,
    /// Show the request and send nothing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Subcommand)]
pub enum MessageCmd {
    /// Read message history, oldest first
    Read(ReadArgs),
    /// Send a message
    Send(SendArgs),
    /// Edit one of your messages
    Edit {
        #[arg(help = TARGET_HELP)]
        target: String,
        /// Id of the message
        message: String,
        /// The new text
        text: Vec<String>,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        mentions: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a message
    Delete {
        #[arg(help = TARGET_HELP)]
        target: String,
        /// Id of the message
        message: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a reaction (or remove yours)
    React {
        #[arg(help = TARGET_HELP)]
        target: String,
        /// Id of the message
        message: String,
        /// An emoji, or name:id for a custom one
        emoji: String,
        /// Remove your reaction instead
        #[arg(long)]
        remove: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum VoiceCmd {
    /// Which voice channel you are in and who is there
    Status,
    /// Join a voice channel (a call in a chat only if one is already running)
    Join {
        /// Voice channel: Server/channel or an id; @name for a running call in a chat
        target: String,
        /// Allow starting a NEW call in a chat: this rings the other people
        #[arg(long)]
        ring: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Leave the voice channel
    Leave {
        #[arg(long)]
        dry_run: bool,
    },
    /// Mute your microphone
    Mute,
    /// Unmute your microphone
    Unmute,
    /// Stop hearing others
    Deafen,
    /// Hear others again
    Undeafen,
}

#[derive(Subcommand)]
pub enum StreamCmd {
    /// List windows and screens that can be shared
    #[command(visible_alias = "sources")]
    List,
    /// Are you sharing, and where
    Status,
    /// Share a window or screen into the voice channel you are in (join one first)
    Start {
        /// Window title (part of it) or an id from `stream list`
        source: String,
        /// Share sound too (off by default)
        #[arg(long)]
        audio: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Stop sharing
    Stop {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum AppCmd {
    /// Create an app with a token and the rights you approve (from flags or a manifest)
    Add(AppAddArgs),
    /// Apps and their rights
    List,
    /// Delete an app: its token stops working at once
    Remove { name: String },
    /// Last entries of the audit log
    Log {
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
    /// Answer what apps ask to do (keep this running in its own window)
    Approve,
    /// Who the daemon thinks you are: the owner, or an app (depends on DISCORDCTL_TOKEN)
    Whoami,
}

#[derive(Args)]
pub struct AppAddArgs {
    /// App name (letters, digits, - and _); optional when --manifest names it
    pub name: Option<String>,
    /// Read the rights from a TOML manifest (see examples/agent.toml)
    #[arg(long, conflicts_with_all = ["list", "read", "write", "voice", "voice_channel", "stream"])]
    pub manifest: Option<PathBuf>,
    /// Lists it may see: servers, channels, chats, friends, me, or *
    #[arg(long, value_name = "WHAT")]
    pub list: Vec<String>,
    /// Chats it may read: @name, Server/channel, Server/*, @*, or *
    #[arg(long, value_name = "TARGET")]
    pub read: Vec<String>,
    /// Chats it may write to (same forms)
    #[arg(long, value_name = "TARGET")]
    pub write: Vec<String>,
    /// Voice actions: join, leave, mute, deafen, status, or *
    #[arg(long, value_name = "ACTION")]
    pub voice: Vec<String>,
    /// Voice channels it may join (same forms as --read)
    #[arg(long, value_name = "TARGET")]
    pub voice_channel: Vec<String>,
    /// Sharing: list, status, stop, window, screen, or *
    #[arg(long, value_name = "ACTION")]
    pub stream: Vec<String>,
    /// Do not ask before each call (dangerous: by default writes, voice and sharing wait for you)
    #[arg(long)]
    pub no_confirm: bool,
}

#[derive(Subcommand)]
pub enum McpCmd {
    /// Create an app for the agent and register the MCP server in its config
    Install {
        /// Which agent to connect (repeatable); asks if omitted
        #[arg(long, value_enum)]
        agent: Vec<AgentKind>,
        /// What the agent may do
        #[arg(long, value_enum, default_value = "read-only")]
        preset: Preset,
        /// Skip the questions
        #[arg(short, long)]
        yes: bool,
    },
    /// Which agents are connected
    Status,
    /// Disconnect an agent: remove it from the config and revoke its token
    Remove {
        #[arg(long, value_enum, required = true)]
        agent: Vec<AgentKind>,
    },
}

#[derive(Subcommand)]
pub enum DaemonCmd {
    /// Start the daemon
    Start,
    /// Stop the daemon (this closes a running Discord, so --force is required)
    Stop {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum DevCmd {
    /// Call a payload method inside Discord: `dev call selfcheck`
    Call {
        method: String,
        /// Parameters as a JSON object
        params: Option<String>,
    },
    /// Reload the Discord page with a payload bundle from disk
    Reload {
        #[arg(long)]
        payload: Option<PathBuf>,
    },
}
