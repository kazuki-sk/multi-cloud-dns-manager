# マルチクラウド DNS 統合管理ツール 設計ドキュメント

本文書は、複数クラウドプロバイダーに跨る DNS（権威 DNS プロバイダー層の冗長化）を統合的に管理する Web UI ベースのツールの要件定義およびアーキテクチャ概要である。 議論経過における決定事項・採用しなかった選択肢の根拠も併記する。

---

## 1\. 概要

### 1.1 製品コンセプト

複数の権威 DNS プロバイダー（Route53 / Cloud DNS / Cloudflare / 国内 DNS 等）に対して、**同じ DNS レコードを一元的な Web UI から管理・同期する**ためのツール。PowerDNS-Admin の発想を「単一バックエンド → 複数クラウド DNS バックエンド」に拡張したもの。

### 1.2 主目的

**DNS プロバイダー自体の冗長化**。単一プロバイダーの障害（例: 2016 年 Dyn DDoS 事件）で名前解決が停止する事態を回避するため、複数の権威 DNS プロバイダーに同じレコードを保持する運用を支援する。

### 1.3 スコープ外（明示的に対象外とする項目）

- インフラ層の冗長化（AWS 落ちたら GCP へ等のフェイルオーバー）  
- IaC・変更管理（Terraform / OctoDNS / DNSControl の領域）  
- DNS レジストラ機能（NS レコードの登録・移管）  
- 上位ドメインの NS 委任管理

---

## 2\. 設計原則

| \# | 原則 | 根拠 |
| :---- | :---- | :---- |
| 1 | **Desired State / Actual State の分離** | reconcile 型アーキテクチャの根幹 |
| 2 | **Least Common Denominator 抽象化** | 冗長化を成立させる範囲 \= 抽象化可能な範囲 |
| 3 | **冗長化の崩壊を許さない** | DNS は eventually consistent で済まない（クリティカル） |
| 4 | **provider 側手動変更を機械的に潰さない** | drift 検出 ≠ 自動上書き |
| 5 | **ツール自身が SPOF にならない** | 同期エンジン障害が DNS 障害を生まない設計 |
| 6 | **キューは内部実装、UI には見せない** | Azure ARM / Route53 LRO と同じ非同期パターン |
| 7 | **プラグイン式での provider 拡張** | アジャイル・段階的なプロバイダー追加 |

---

## 3\. 機能要件

### 3.1 基本機能

- 複数 DNS プロバイダーの統合管理（一つの UI で全 provider を操作）  
- DNS レコードの CRUD（A / AAAA / CNAME / MX / TXT / NS / SRV / CAA など標準型）  
- 対応レコード種別は **接続中の全 provider が共通サポートするもの**に動的に絞り込まれる  
- Zone のインポート（既存の provider 上のゾーンを取り込む）※詳細は将来検討

### 3.2 冗長化・同期機能

- 1 つの論理レコードを N 個の provider に展開  
- 書き込み一貫性モデル：**Best-effort \+ Reconciliation**  
- 同期状態の継続的監視（drift detection）  
- 同期失敗時の自動リトライ（指数バックオフ）

### 3.3 安全機構

| 機構 | MVP | 説明 |
| :---- | :---- | :---- |
| Dry-run / Preview | ✅必須 | 適用前に全 provider 分の差分プレビュー |
| Pre-flight validation | ✅必須 | 構文・NS 整合性・明らかな typo 検出 |
| 自動ロールバック | ✅必須 | 即時失敗（\<30s）に対する自動復旧 |
| 手動ロールバック | ✅必須 | 任意時点での ChangeSet 単位の復元 |
| クラッシュ復旧 | ✅必須 | 'applying' 状態のままツールが落ちた場合の resume |
| Canary write | 🔵後回し | 1 provider に先行書き込み → 検証 → 残りに展開 |
| TTL 戦略支援 | 🔵後回し | 変更予定レコードの事前 TTL 短縮 |

### 3.4 ロールバックポリシー（フェーズ別）

| フェーズ | 状況 | デフォルト挙動 |
| :---- | :---- | :---- |
| 即時失敗 | 書き込み直後（30秒以内）に失敗検出 | **自動ロールバック** |
| 遅延 drift | 後の observation で不一致検出 | **アラート \+ 人間判断** |
| 永続的失敗 | リトライしても収束しない | **アラート \+ ChangeSet 凍結** |

### 3.5 認証認可

- MVP は local 認証のみ  
- データモデル上は外部 IdP 連携を hook として持つ  
- 将来: LDAP / Entra ID / OIDC / SAML / OAuth  
- 権限は zone 単位の ACL（viewer / editor / admin の 3 段階固定で開始）  
- API トークンによる programmatic access

### 3.6 監査・履歴

- 全レコード変更を不変ログとして保持  
- 保持期間はユーザー定義（N 日、または無期限）  
- 削除レコードは tombstone として保持（完全削除しない）

---

## 4\. 非機能要件

| 項目 | 要件 |
| :---- | :---- |
| **可用性** | ツール障害が DNS 解決障害に直結しないこと（権威 DNS は各 provider が独立に稼働しているため、ツール停止中も解決は継続する） |
| **整合性** | provider 間の値の不一致は検出・通知される。自動上書きはしない |
| **観測性** | 同期状態・エラー率・drift 件数をメトリクスとして公開 |
| **スケール** | 本番・複数チーム運用に耐える（具体的な数値要件は別途定義） |
| **セキュリティ** | provider 認証情報は secret store に保管。UI には平文露出しない |
| **拡張性** | 新規 provider 追加はプラグイン実装のみで済むこと（コア改修不要） |

---

## 5\. アーキテクチャ

### 5.1 コンポーネント概要

┌─────────────────────────────────────────────────────────┐

│                       Web UI (SPA)                       │

└─────────────────────────────┬───────────────────────────┘

                              │ REST/WS

┌─────────────────────────────▼───────────────────────────┐

│                      API Server                          │

│  \- 認証認可                                              │

│  \- ChangeSet 受付・バリデーション                        │

│  \- 状態の購読 (SSE/WS)                                   │

└──────┬──────────────────────────────────┬───────────────┘

       │                                  │

       ▼                                  ▼

┌──────────────┐                  ┌────────────────────────┐

│   Database    │                  │  Reconcile Worker(s)   │

│  \- Desired   │◀────state────────│  \- Apply Phase         │

│  \- Actual    │     read/write    │  \- Observe Phase       │

│  \- History   │                   │  \- Retry Phase         │

└──────────────┘                   │  \- Rollback Phase      │

                                   └──────────┬─────────────┘

                                              │

                                              ▼

                                   ┌──────────────────────┐

                                   │  Provider Adapters    │

                                   │  ├ Route53            │

                                   │  ├ Cloudflare         │

                                   │  ├ Google Cloud DNS   │

                                   │  └ ...                │

                                   └──────────┬───────────┘

                                              │ provider API

                                              ▼

                                   外部 DNS プロバイダー群

### 5.2 データモデル

#### Zone（論理ゾーン）

\- id

\- name              (例: example.com)

\- default\_ttl

\- owner\_team        (FK → Team)

\- created\_at, updated\_at

#### ProviderBinding（論理ゾーン ↔ provider の紐付け）

\- id

\- zone\_id           (FK → Zone)

\- provider\_type     (route53 | cloudflare | gcloud | ...)

\- provider\_zone\_id  (各社の内部 ID, 例: Route53 の "Z1234ABC")

\- credentials\_ref   (secret store 参照キー)

\- status            (active | paused | error)

#### Record（論理レコード \= Desired State）

\- id

\- zone\_id           (FK → Zone)

\- name              (www / @ / \*)

\- type              (A | AAAA | CNAME | MX | TXT | NS | ...)

\- values            (型に応じた構造のリスト)

\- ttl

\- desired\_hash      (高速比較用)

\- deleted\_at        (tombstone 用, NULL なら有効)

#### SyncState（Actual State \+ 同期状況, record × provider\_binding）

\- record\_id, provider\_binding\_id   (複合キー)

\- last\_observed\_value

\- last\_observed\_at

\- status            (in\_sync | drift | sync\_failed | syncing)

\- last\_error

\- retry\_count

#### ChangeSet（変更のトランザクション単位）

\- id

\- created\_by        (FK → User)

\- created\_at

\- description

\- status            (draft | validated | applying | applied

                     | rolling\_back | rolled\_back | rollback\_failed

                     | frozen)

\- rollback\_policy   (auto | manual | frozen\_on\_failure)

**粒度方針**: 1 ChangeSet は複数レコードの同時編集を含める。DNS 移行時の関連レコード群を atomic に扱える。

#### ChangeSetItem（ChangeSet 内の個別レコード変更）

\- changeset\_id, record\_id          (複合キー)

\- operation         (create | update | delete)

\- before\_value      (ロールバック用スナップショット)

\- after\_value

#### ChangeSetApplication（ChangeSet × provider\_binding の適用結果）

\- changeset\_id, provider\_binding\_id   (複合キー)

\- status            (pending | success | failed | rolled\_back)

\- attempted\_at, error\_message

\- retry\_count

#### History（不変ログ）

\- id

\- record\_id, changeset\_id

\- operation, before\_value, after\_value

\- timestamp, user\_id

#### User

\- id, email, display\_name

\- auth\_source       (local | ldap | oauth | saml | oidc)

\- auth\_source\_id    (外部 IdP 側ユニーク ID, nullable)

\- status            (active | disabled)

\- created\_at, last\_login\_at

#### Team

\- id, name, description

\- created\_at

#### TeamMembership

\- user\_id, team\_id  (複合キー)

\- role\_in\_team      (member | admin)

#### ZoneACL（zone × subject × role）

\- zone\_id

\- subject\_type      (team | user)

\- subject\_id

\- role              (viewer | editor | admin)

#### APIToken

\- id, name, hashed\_token

\- owner\_user\_id (or owner\_team\_id)

\- scopes

\- expires\_at

### 5.3 ChangeSet 状態遷移

   draft

     │ (バリデーション)

     ▼

  validated

     │ (適用開始)

     ▼

  applying ──── 全 provider 成功 ────▶ applied

     │

     ├── 即時失敗 (\<30s, auto) ──▶ rolling\_back ──▶ rolled\_back

     │                                          └──▶ rollback\_failed (frozen)

     │

     └── 永続失敗 / 部分失敗 ────▶ frozen（要人手介入）

### 5.4 Reconcile Loop

4 つの位相を独立に動作させる。

#### Apply Phase

for each ChangeSetItem in ChangeSet:

  for each ProviderBinding in Zone:

    adapter.upsertRecord(creds, providerZoneId, record)

    → ChangeSetApplication に結果記録

集計：

  \- 全成功            → applied

  \- 即時失敗 ≥ 1 (auto) → rollback 開始

  \- リトライ枠超過    → frozen

#### Observe Phase

for each ProviderBinding:

  records \= adapter.listRecords(...)

  diff against Desired State

  update SyncState:

    一致     → in\_sync

    不一致   → drift（自動上書きせず、アラート）

    取得失敗 → sync\_failed

#### Retry Phase

- status=sync\_failed なものを指数バックオフで再試行  
- N 回失敗後は frozen へ遷移し、人手介入待ち

#### Rollback Phase

- ChangeSetItem.before\_value を使って各 provider を復元  
- ロールバック自体が失敗した場合は rollback\_failed (frozen) で停止し、人手介入

### 5.5 並行制御

**Zone 単位の FIFO キュー**。

- DNS の変更頻度は秒間何件にもならない  
- 人間判断を挟む場面が多い  
- シンプルで実装コストが低い

UI 側にはキューの存在を見せない（Azure ARM / Route53 と同じ非同期パターン）。

### 5.6 クラッシュ復旧

- ツール起動時に 'applying' 状態の ChangeSet を検出  
- 各 ChangeSetApplication 状態を provider 側に問い合わせて再構築  
- 未完了分を resume

これは MVP の必須要件（原則 5「ツール自身が SPOF にならない」の実装）。

### 5.7 Observe 頻度

- デフォルト 5 分間隔のポーリング  
- ChangeSet 適用直後の即時 observe  
- provider 別にレート制限考慮（後述の plugin constraints で表現）

### 5.8 Provider Plugin I/F

各 provider 実装が満たす契約：

interface ProviderAdapter {

  // メタデータ

  providerId(): string

  supportedRecordTypes(): RecordType\[\]

  constraints(): { min\_ttl, max\_ttl, name\_max\_length, ... }

  // Zone 操作

  listZones(creds): ProviderZone\[\]

  getZone(creds, providerZoneId): ProviderZoneDetail

  // Record 操作（idempotent 必須）

  listRecords(creds, providerZoneId): ProviderRecord\[\]

  upsertRecord(creds, providerZoneId, record): Result

  deleteRecord(creds, providerZoneId, recordKey): Result

  // バリデーション

  validateRecord(record): ValidationResult

}

`constraints()` を実装させることで、ツール側が「このプロバイダー組み合わせで使える機能の交集合」を動的に計算できる。新規プロバイダー追加時に既存ロジックの改修不要。

---

## 6\. UI 設計

### 6.1 主要画面

| \# | 画面 | 役割 |
| :---- | :---- | :---- |
| 1 | Zone 一覧 (Dashboard) | 各 zone の health 集約表示 |
| 2 | Zone 詳細 (Record List) | レコード一覧 \+ 各レコードの sync 状態 |
| 3 | ChangeSet 詳細 | 状態遷移可視化 \+ 各 provider 適用状況 \+ diff |
| 4 | Drift View | actual ≠ desired のレコード一覧と解消アクション |
| 5 | History / Audit | 時系列の全変更ログ |

### 6.2 状態語彙の翻訳（内部 → UI 表示）

| 内部状態 | UI 表示 | 性格 |
| :---- | :---- | :---- |
| draft | 下書き | 情報 |
| validated | （非表示、内部のみ） | — |
| applying | 適用中 | 情報 |
| applied | 反映済み | 情報 |
| rolling\_back | ロールバック中 | 情報 |
| rolled\_back | ロールバック完了 | 情報 |
| rollback\_failed | **要対応：ロールバック失敗** | 警告 |
| frozen | **要対応：停止中** | 警告 |

「要対応」は色・バナー・通知で明確に区別。それ以外は情報表示として穏やかに。

### 6.3 Sync indicator 集約方針

1 レコードが N プロバイダーに展開されているとき、UI 上では**最悪状態を集約表示**（in\_sync \< syncing \< drift \< sync\_failed）し、展開で provider 別詳細を表示する。

理由：表形式で並んだとき視覚ノイズが少なく、要対応のものが目立つ。

### 6.4 Drift 解消の UX

| アクション | 内容 | 確認 |
| :---- | :---- | :---- |
| 取り込み | actual を新 desired として採用 | 1 段階確認 |
| 強制 | desired で actual を上書き | **2 段階確認**（本番事故防止） |
| 保留 | 情報として記録のみ | 確認なし |

### 6.5 Apply 中の表示方針

**慎重表示**を採用。UI 上は「適用中」を維持し、全 provider への反映が確認できた時点で「適用済み」へ遷移。

理由：DNS のクリティカリティを考えると、誤って「終わった」と思わせるリスクは負えない。

### 6.6 全体 UX 原則

- ユーザーが見るのは ChangeSet の lifecycle 状態のみ。キュー位置は見せない  
- 「あなたは X 番目」のような表示はしない  
- 全書き込み API は非同期。submit → ChangeSet ID 返却 → 状態購読  
- 観測指標は「平均応答時間」ではなく「状態遷移までの時間」

---

## 7\. MVP スコープ

### 7.1 MVP に含めるもの

- Web UI（zone 一覧、レコード CRUD、ChangeSet 詳細、Drift View、History）  
- 標準レコード型のサポート（A / AAAA / CNAME / MX / TXT / NS）  
- 2–3 provider の adapter 実装（例: Route53 / Cloudflare / Cloud DNS）  
- ChangeSet による atomic 変更（複数レコード同時編集）  
- Best-effort \+ Reconciliation の書き込み  
- Dry-run, Pre-flight validation  
- 自動・手動ロールバック  
- クラッシュ復旧  
- local 認証 \+ Team \+ ZoneACL \+ API トークン  
- 不変 History 保持（保持期間ユーザー設定）

### 7.2 MVP には含めないもの（将来）

- SSO（LDAP / Entra ID / OIDC / SAML）  
- 外部 group → 内部 team の自動マッピング  
- record type 別 / provider 別の細粒度権限  
- 承認フロー（管理者承認が必要な変更）  
- Service account（人ではない identity）  
- Canary write  
- TTL 戦略支援  
- 既存ゾーンの一括インポート（手作業同等まで）  
- 通知連携（Slack / Email / PagerDuty）の本格対応  
- record 単位 ACL  
- Org / マルチテナンシー（SaaS 化を見据える場合）

---

## 8\. 論点と決定事項

| \# | 論点 | 状態 |
| :---- | :---- | :---- |
| 1 | 既存ゾーンの初期 import フロー設計（※8.1参照） | 決定済み |
| 2 | 通知（frozen / rollback\_failed / drift 検出）の連携先と粒度（※8.2参照） | 決定済み |
| 3 | API 設計（REST / GraphQL / 両方）（※8.3参照） | 決定済み |
| 4 | 技術スタック選定（言語・フレームワーク・DB） | 決定済み |
| 5 | デプロイトポロジ（worker と API の分離、HA 構成）（※8.5参照） | 決定済み |
| 6 | 認証情報（provider creds）の暗号化・ローテーション戦略（※8.6参照） | 決定済み |
| 7 | provider レート制限への耐性設計（※8.7参照） | 決定済み |
| 8 | observe loop の効率化（webhook 対応 provider との連携）（※8.8参照） | 決定済み |
| 9 | 大量レコード zone でのスケール特性（※8.9参照） | 決定済み |
| 10 | DNSSEC 対応の方針（※8.10参照） | 決定済み |

---

## 9\. 用語集

| 用語 | 定義 |
| :---- | :---- |
| **Desired State** | ツールが「こうあるべき」と考える状態。ユーザーが UI で編集する対象 |
| **Actual State** | 各 provider に実際に登録されている状態。observe によって取得 |
| **Drift** | Desired State と Actual State の不一致 |
| **ChangeSet** | 1 回の編集をトランザクション的にまとめる単位。複数レコード変更を含める |
| **ProviderBinding** | 論理 zone と provider 上の zone の対応関係 |
| **Reconcile Loop** | desired と actual の差分を継続的に解消する処理ループ |
| **Tombstone** | 削除されたが DB 上に残されたレコード（ロールバック・監査用） |
| **Frozen** | 自動処理を停止し人手介入を待つ状態 |
| **LCD（Least Common Denominator）** | 全 provider で共通サポートされる機能・レコード型の集合 |

---

## 8.1 既存ゾーンの初期 import フロー

| 項目 | 決定内容 |
| :---- | :---- |
| 主なユースケース | 複数 provider にバラバラに存在するゾーンの統合。単一 provider からの移行も対応 |
| MVP スコープ | レコードの同期まで（NS 委任・レジストラ操作は対象外） |
| provider 間の差分 | 差分を提示してユーザーが手動解決 |
| 解決粒度 | レコード単位で個別に選択 |
| import 中の Zone 操作 | 通常操作と並行可能（ロックしない） |
| import 中に編集されたレコード | import 対象から除外（解決済み扱い） |

## 8.2 通知連携

| 項目 | 決定内容 |
| :---- | :---- |
| MVP 対応 | WebUI 内通知（ベルアイコン \+ 通知一覧） |
| 対象イベント | frozen / rollback\_failed / drift 検出 |
| 外部連携 | Slack / Email / PagerDuty 等は将来対応 |

## 8.3 API 設計

| 項目 | 決定内容 |
| :---- | :---- |
| 方式 | RESTful API |
| 理由 | 状態遷移が明確な ChangeSet ライフサイクルと相性が良く、SSE/WebSocket との共存もシンプル |
| リアルタイム通知 | SSE または WebSocket を REST と併用 |

## 8.4 技術スタック選定

| 項目 | 決定内容 |
| :---- | :---- |
| バックエンド言語 | Rust |
| Web フレームワーク | axum |
| 非同期ランタイム | tokio |
| DB クライアント | sqlx |
| DB（デフォルト） | SQLite |
| DB（オプション） | PostgreSQL / MySQL |
| フロントエンド | React（SPA） |
| フロントビルド | Vite |
| パッケージ管理 | cargo（BE） / npm（FE） |

## 8.5 デプロイトポロジ

| 項目 | 決定内容 |
| :---- | :---- |
| MVP | API Server と Reconcile Worker を同一プロセスで動作 |
| 将来 | インターフェースを切っておき、必要に駆られたら別プロセスに分離可能とする |

## 8.6 認証情報の暗号化・ローテーション

| 項目 | 決定内容 |
| :---- | :---- |
| 保管方式 | Envelope Encryption（AES-256-GCM）。encrypted\_blob \+ encrypted\_DEK を DB に保存 |
| KEK 管理（MVP） | 環境変数で注入 |
| KEK 管理（将来） | KeyProvider プラグインで差し替え可能（Vault / KMS 等） |
| credential ローテーション | ユーザーが手動再登録。ツールは自動化しない |
| 切り替えタイミング | 即時（次の Worker 実行から新キーを使用） |
| KEK ローテーション | MVP スコープ外 |

## 8.7 provider レート制限への耐性

| 項目 | 決定内容 |
| :---- | :---- |
| 方針 | ProviderAdapter の constraints() に rate\_limit フィールドを定義。Worker 側が従う |
| デフォルト | null（制限なし）。各 provider の adapter が必要に応じて設定 |

## 8.8 observe loop の効率化（webhook）

| 項目 | 決定内容 |
| :---- | :---- |
| 方針 | webhook 対応は Provider Adapter に委譲 |
| 設定 | ProviderBinding 単位でユーザーが webhook 有効/無効を設定可能 |
| 非対応 provider | ポーリング（デフォルト 5 分）を継続 |

## 8.9 大量レコード zone のスケール特性

現時点のユースケース外。数万レコード規模の運用は想定しない。問題が生じた段階で対処する。

## 8.10 DNSSEC 対応

| 項目 | 決定内容 |
| :---- | :---- |
| 完全実装 | MVP スコープ外 |
| 将来への余地 | constraints() に dnssec\_supported フラグを持たせる |

## 10\. 変更履歴

| 版 | 日付 | 内容 |
| :---- | :---- | :---- |
| 0.1 | 初版 | 設計議論からの初版作成 |
| 0.2 | 2025-05-12 | オープン論点（\#1〜\#10）の決定事項を反映。技術スタック（Rust / axum / tokio / sqlx / React / Vite）確定。 |

