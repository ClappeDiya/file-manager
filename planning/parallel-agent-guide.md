# Team Agent Parallel Execution Guide

## How to Run Multiple Claude Code Agents in Parallel

Instead of one agent doing all 70 tasks sequentially, split work across
multiple Claude Code instances in separate tmux panes. Each agent reads
the same spec files but executes only its assigned tasks.

---

## Execution Waves

### Wave 1: Foundation (2 agents)

AGENT 1 - Rust Foundation (tmux pane 1):
Assigned tasks: T-001, T-002, T-003, T-004
After T-001 completes, signal Agent 2 to start.

AGENT 2 - Frontend Foundation (tmux pane 2):
Assigned tasks: T-005 (waits for T-001), T-006, T-008, T-014
Depends on: T-001 from Agent 1

### Wave 2: File Manager + Transfer (2 agents)

AGENT 3 - File Manager Core (tmux pane 1):
Assigned tasks: T-007, T-009, T-010, T-011, T-012, T-013

AGENT 4 - Transfer Engine (tmux pane 2):
Assigned tasks: T-015, T-016, T-017, T-018, T-019
Depends on: T-003, T-010

### HARDENING CHECKPOINT 1 after Wave 2

### Wave 3: Connectors (3-4 agents in parallel)

AGENT 5 - Protocol Connectors (tmux pane 1):
Assigned tasks: T-020 (SFTP), T-021 (FTP), T-022 (WebDAV)
Depends on: T-015, T-019

AGENT 6 - Network Connectors (tmux pane 2):
Assigned tasks: T-023 (SMB), T-024 (NFS), T-025 (Drive-to-Drive)
Depends on: T-015, T-019

AGENT 7 - Cloud Connectors (tmux pane 3):
Assigned tasks: T-026 (S3), T-027 (GDrive), T-028 (Dropbox), T-029 (OneDrive), T-030 (B2)
Depends on: T-015, T-019

AGENT 8 - Peer and Migration (tmux pane 4):
Assigned tasks: T-031, T-032, T-033
Depends on: T-015, T-019, T-025

### HARDENING CHECKPOINT 2 after Wave 3

### Wave 4: Compatibility + Sync (2 agents in parallel)

AGENT 9 - Compatibility Engine (tmux pane 1):
Assigned tasks: T-034, T-035, T-036, T-037, T-038
Depends on: T-002

AGENT 10 - Sync Engine (tmux pane 2):
Assigned tasks: T-039, T-040, T-041, T-042
Depends on: T-015, T-003

### HARDENING CHECKPOINT 3 after Wave 4

### Wave 5: AI + Security + Enterprise (3 agents in parallel)

AGENT 11 - AI and Terminal (tmux pane 1):
Assigned tasks: T-043, T-044, T-045, T-046
Depends on: T-015, T-039

AGENT 12 - Security and Encryption (tmux pane 2):
Assigned tasks: T-047
Depends on: T-019, T-003

AGENT 13 - Enterprise (tmux pane 3):
Assigned tasks: T-048, T-049, T-050, T-051, T-052, T-053
Depends on: T-001

### HARDENING CHECKPOINT 4 after Wave 5

### Wave 6: UX + CLI + Integration (3 agents in parallel)

AGENT 14 - Simple Mode and UX (tmux pane 1):
Assigned tasks: T-054, T-055, T-056, T-057

AGENT 15 - CLI and API (tmux pane 2):
Assigned tasks: T-058, T-059, T-060, T-061, T-062, T-063

AGENT 16 - Packaging and Launch (tmux pane 3):
Assigned tasks: T-064, T-065

### Wave 7: Final Validation (1-2 agents)

AGENT 17 - E2E and Security Audit:
Assigned tasks: T-066, T-067

AGENT 18 - Docs and Release:
Assigned tasks: T-068, T-069, T-070

### HARDENING CHECKPOINT 5 before launch

---

## Agent Launch Template

For each agent, paste the following into its tmux pane, replacing
the ASSIGNED_TASKS and DEPENDS_ON lines:

---

Read the file CLAUDE-CODE-INSTRUCTION.md for full project context.
Then read all specification files listed in Step 1 of that instruction.

You are one agent in a parallel team. Your role:

ASSIGNED TASKS: [list task IDs, e.g., T-020, T-021, T-022]
DEPENDENCIES: [list prerequisite task IDs that must be complete]
WAVE: [wave number]

RULES:
- Read planning/build-sequence.md for your task details
- Read ai-agents/global-policy.yaml for governance rules
- Read the agent YAMLs listed in Load Agents for each of your tasks
- Follow all architecture rules from CLAUDE-CODE-INSTRUCTION.md
- Do NOT implement tasks outside your assignment
- Do NOT modify files owned by another agent without coordination
- When your tasks are complete, report what was built and any integration notes

AUTONOMOUS DELIVERY RULE: Do not stop for interim approval.
Continue implementing until all your assigned tasks are complete.
Only stop when done or hitting a true hard blocker.

Begin by reading your task definitions from planning/build-sequence.md,
then start implementing your first assigned task.

---

## Coordination Rules

1. Agents must not edit the same file simultaneously
2. Foundation agents (Wave 1) must complete before Wave 2 starts
3. Each wave waits for its listed dependencies
4. Hardening checkpoints are synchronization points -- all agents in the wave
   must complete before the next wave begins
5. If Agent A creates an interface that Agent B depends on, Agent A
   must complete and commit before Agent B starts consuming it
6. Shared crates (core types, error types) should be built by one agent
   in Wave 1 and treated as read-only by subsequent agents

## Tmux Setup

tmux new-session -s filemanager
tmux split-window -h
tmux split-window -v
tmux select-pane -t 0
tmux split-window -v

This gives you 4 panes. Open Claude Code in each pane:
  cd /Users/md/Documents/Md/FileManager
  claude

Then paste the agent-specific prompt into each pane.

## Maximum Parallel Agents Per Wave

- Wave 1: 2 agents
- Wave 2: 2 agents
- Wave 3: 4 agents (maximum parallelism)
- Wave 4: 2 agents
- Wave 5: 3 agents
- Wave 6: 3 agents
- Wave 7: 2 agents

Total across entire build: 18 agent sessions (not all running simultaneously)
Maximum concurrent at any time: 4 agents (Wave 3)
