# Unified File Operations Platform

**Cross-Platform File Manager, Transfer, Sync & Governance**

## Project Structure

```
FileManager/
├── README.md                          ← This file
├── docs/                              ← Product specifications
│   ├── PRD-V5-Definitive.md           ← Master PRD (markdown)
│   ├── AI-Agent-Governance-Addendum.md ← Agent governance spec
│   └── *.docx                         ← Formatted documents
├── ai-agents/                         ← AI agent configurations
│   ├── README.md                      ← Agent loading guide
│   ├── global-policy.yaml             ← Shared governance rules
│   ├── agent-index.yaml               ← Agent registry
│   ├── core/                          ← 11 core agents
│   ├── support/                       ← 8 support agents
│   └── workflows/                     ← Task bundle templates
└── planning/                          ← Delivery planning
    ├── sprint-plan.md                 ← 42-sprint plan
    └── epics-summary.md               ← Epic-to-sprint mapping
```

## Tech Stack
- **Desktop:** Tauri 2 + Rust core + React/TypeScript/Vite
- **Admin Console:** Next.js + TypeScript
- **Key Libraries:** TanStack Table/Virtual/Query, Zustand, shadcn/ui, React Aria

## Quick Start
1. Read `docs/PRD-V5-Definitive.md` for full product spec
2. Read `ai-agents/README.md` for agent loading instructions
3. Load `ai-agents/global-policy.yaml` + specific agent for any task

## Document Versions
- PRD: V5 (Definitive)
- Agent Governance: v1.0
- Agent Pack: v1.0
- Sprint Plan: 42 sprints, 8 phases, 21 months
