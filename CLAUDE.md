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

# 現在の実装状況

## バックエンド（完了）
- crates/core: ドメイン型・ProviderAdapter trait・Envelope Encryption
- crates/db: sqlxマイグレーション（zones/provider_bindings/desired_records/changesets/sync_states）
- crates/api: axumサーバー・zones/records/providers/changesets エンドポイント
- crates/worker: Apply/Observe/Retry Phase
- crates/adapters/route53: Route53 ProviderAdapter
- crates/adapters/azuredns: AzureDNS ProviderAdapter（Service Principal認証）
- crates/adapters/gcloud: 未実装

## フロントエンド（進行中）
- React + Vite + TypeScript + Tailwind + React Query
- Viteプロキシ設定済み（/api → localhost:8080）
- ZonesPage: 完了
- ZoneDetailPage: 完了（レコード一覧・追加）
- ProvidersPage: 実装中
- ChangesetsPage: 未実装

## 起動方法
# バックエンド
cd backend
DATABASE_URL=sqlite:dns_manager.db MASTER_KEY=$(openssl rand -base64 32) cargo run

# フロントエンド
cd frontend
npm run dev
