use serde::{Deserialize, Serialize};

/// ChangeSet のライフサイクル状態（設計書 §5.3 の状態遷移に準拠）。
///
/// 遷移図:
///   Draft → Validated → Applying → Applied
///                          │
///                          ├─(即時失敗 <30s, auto)─→ RollingBack → RolledBack
///                          │                                    └→ RollbackFailed  ← terminal/frozen
///                          └─(永続失敗/部分失敗)──→ Frozen  ← terminal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    Draft,
    Validated,
    Applying,
    Applied,
    RollingBack,
    RolledBack,
    /// ロールバック自体が失敗。人手介入が必要（UI: "要対応"）。
    RollbackFailed,
    /// 自動処理を停止し人手介入を待つ状態（UI: "要対応"）。
    Frozen,
}

impl ChangeSetStatus {
    /// この状態から遷移可能かどうかを返す。
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Validated)
                | (Self::Validated, Self::Applying)
                | (Self::Applying, Self::Applied)
                | (Self::Applying, Self::RollingBack)
                | (Self::Applying, Self::Frozen)
                | (Self::RollingBack, Self::RolledBack)
                | (Self::RollingBack, Self::RollbackFailed)
        )
    }

    /// これ以上の自動遷移が発生しない終端状態。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::RolledBack | Self::RollbackFailed | Self::Frozen
        )
    }

    /// UI で警告表示（"要対応"）すべき状態。
    pub fn requires_intervention(self) -> bool {
        matches!(self, Self::RollbackFailed | Self::Frozen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_transitions() {
        assert!(ChangeSetStatus::Draft.can_transition_to(ChangeSetStatus::Validated));
        assert!(ChangeSetStatus::Validated.can_transition_to(ChangeSetStatus::Applying));
        assert!(ChangeSetStatus::Applying.can_transition_to(ChangeSetStatus::Applied));
    }

    #[test]
    fn rollback_path() {
        assert!(ChangeSetStatus::Applying.can_transition_to(ChangeSetStatus::RollingBack));
        assert!(ChangeSetStatus::RollingBack.can_transition_to(ChangeSetStatus::RolledBack));
        assert!(ChangeSetStatus::RollingBack.can_transition_to(ChangeSetStatus::RollbackFailed));
    }

    #[test]
    fn frozen_path() {
        assert!(ChangeSetStatus::Applying.can_transition_to(ChangeSetStatus::Frozen));
    }

    #[test]
    fn invalid_transitions_rejected() {
        assert!(!ChangeSetStatus::Draft.can_transition_to(ChangeSetStatus::Applying));
        assert!(!ChangeSetStatus::Applied.can_transition_to(ChangeSetStatus::Draft));
        assert!(!ChangeSetStatus::Frozen.can_transition_to(ChangeSetStatus::Applying));
    }

    #[test]
    fn terminal_states() {
        assert!(ChangeSetStatus::Applied.is_terminal());
        assert!(ChangeSetStatus::RolledBack.is_terminal());
        assert!(ChangeSetStatus::RollbackFailed.is_terminal());
        assert!(ChangeSetStatus::Frozen.is_terminal());
        assert!(!ChangeSetStatus::Applying.is_terminal());
    }

    #[test]
    fn intervention_required() {
        assert!(ChangeSetStatus::RollbackFailed.requires_intervention());
        assert!(ChangeSetStatus::Frozen.requires_intervention());
        assert!(!ChangeSetStatus::Applied.requires_intervention());
    }
}
