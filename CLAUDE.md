# プロジェクト概要
マルチクラウドDNS統合管理ツール。設計書は docs/design.md を参照。

# 技術スタック
- Backend: Rust / axum / tokio / sqlx
- DB: SQLite（デフォルト）
- Frontend: React / Vite

# 設計原則（必ず守ること）
- ProviderAdapterはプラグイン式（既存コア改修不要）
- ChangeSetの状態遷移は設計書の図に従う
- DBには平文credentialを保存しない（Envelope Encryption）

# ディレクトリ構成
.
├── CLAUDE.md
├── backend
│   ├── Cargo.lock
│   ├── Cargo.toml
│   └── crates
│       ├── adapters
│       │   ├── azuredns
│       │   ├── gcloud
│       │   └── route53
│       ├── api
│       ├── core
│       │   ├── Cargo.toml
│       │   └── src
│       │       ├── changeset.rs
│       │       ├── lib.rs
│       │       ├── provider.rs
│       │       └── record.rs
│       ├── db
│       └── worker
├── docs
│   └── design.md
└── frontend

