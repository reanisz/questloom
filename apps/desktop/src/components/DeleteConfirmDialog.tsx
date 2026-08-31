/**
 * タスク削除の確認ダイアログ。
 *
 * 削除はソフトデリートなので取り返しはつくが、ボードから消える操作なので
 * 一度だけ止めて確認する(復元はヘッダの「削除済み」から)。
 *
 * ドロワーの「🗑 削除」とカードの右クリックメニューの「🗑 削除」が同じものを出すよう、
 * 枠 ([`ModalShell`]) とセレクタ (`aria-label="タスクを削除"` /
 * `data-testid="confirm-delete"`) ごとここに集約してある。
 */

import { ModalShell } from "./ModalShell";

interface Props {
  /** 消そうとしているタスクのタイトル。取り違え防止に本文へ出す。 */
  title: string;
  /** 削除の実行中。真の間はボタンを止め、ダイアログも閉じない。 */
  busy: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

export function DeleteConfirmDialog({ title, busy, onConfirm, onClose }: Props) {
  return (
    <ModalShell
      title="タスクを削除"
      className="modal-sm"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" disabled={busy} onClick={onClose}>
            キャンセル
          </button>
          <button
            type="button"
            className="btn btn-danger"
            data-testid="confirm-delete"
            disabled={busy}
            data-autofocus
            onClick={onConfirm}
          >
            削除
          </button>
        </>
      }
    >
      <p className="confirm-target">{title}</p>
      <p className="muted">
        ボードから消えますが、ヘッダの「削除済み」からいつでも復元できます。子タスクは削除されません。
      </p>
    </ModalShell>
  );
}
