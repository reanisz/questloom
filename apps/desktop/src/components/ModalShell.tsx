/**
 * モーダルダイアログの外枠。
 *
 * scrim(背景の暗幕)+ ヘッダ(タイトルと閉じるボタン)+ 本文 + フッタという
 * 共通の骨格に加えて、次のアクセシビリティ上の面倒を 1 箇所で引き受ける。
 *
 * - `role="dialog"` / `aria-modal` / `aria-label`
 * - 初期フォーカス(`data-autofocus` の要素、無ければ最初のフォーカス可能要素)
 * - フォーカストラップ(Tab / Shift+Tab がダイアログの外へ出ない)
 * - 閉じたときに、開く前にフォーカスしていた要素へ戻す
 * - Esc で閉じる([`useEscapeKey`] のレイヤースタック経由。最前面だけが閉じる)
 *
 * `busy` の間は scrim クリック・✕・Esc のいずれでも閉じない(実行中の取り消しは
 * フッタの「キャンセル」に任せる)。Esc はこのレイヤーで止まるので、後ろの
 * ドロワーが道連れに閉じることもない。
 */

import {
  useEffect,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";

import { useOccludePane } from "../browserPane";
import { ESC_LAYER, useEscapeKey } from "../keyboard";

/** フォーカスを当てられる要素のセレクタ。 */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** 実際に見えているフォーカス可能要素を、DOM 順に返す。 */
function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) => element.getClientRects().length > 0,
  );
}

interface Props {
  /** ヘッダに出す見出し。`aria-label` にも使う。 */
  title: string;
  /** 閉じる要求(✕ / scrim クリック / Esc)。 */
  onClose: () => void;
  /** 実行中。真の間は閉じる操作を受け付けない。 */
  busy?: boolean;
  /** ダイアログ本体に足すクラス(例: `"modal-sm"`)。 */
  className?: string;
  /** フッタの中身。省略するとフッタ自体を出さない。 */
  footer?: ReactNode;
  children: ReactNode;
}

export function ModalShell({ title, onClose, busy = false, className, footer, children }: Props) {
  const dialogRef = useRef<HTMLDivElement>(null);

  // 内蔵ブラウザペイン(子 webview)は HTML の上に描かれるので、開いている間は隠す。
  // そうしないとダイアログがペインの後ろに回り込んで操作できなくなる。
  useOccludePane();

  // 実行中は閉じない。Esc はこのレイヤーで止める(下のドロワーへ渡さない)。
  useEscapeKey(() => {
    if (!busy) onClose();
  }, { priority: ESC_LAYER.modal });

  useEffect(() => {
    const dialog = dialogRef.current;
    const restoreTo = document.activeElement;

    if (dialog) {
      const preferred = dialog.querySelector<HTMLElement>("[data-autofocus]");
      (preferred ?? focusables(dialog)[0] ?? dialog).focus();
    }

    return () => {
      // 閉じたら開く前の要素へフォーカスを返す(消えていたら何もしない)。
      if (restoreTo instanceof HTMLElement && restoreTo.isConnected) restoreTo.focus();
    };
  }, []);

  /** Tab をダイアログ内で循環させる。 */
  const trapFocus = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const items = focusables(dialog);
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === dialog)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <>
      <div className="modal-scrim" onClick={() => !busy && onClose()} />
      <div
        ref={dialogRef}
        className={className ? `modal ${className}` : "modal"}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onKeyDown={trapFocus}
      >
        <header className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={onClose}>
            ✕ 閉じる
          </button>
        </header>

        <div className="modal-body">{children}</div>

        {footer && <footer className="modal-footer">{footer}</footer>}
      </div>
    </>
  );
}
