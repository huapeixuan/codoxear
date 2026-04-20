import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";

interface SessionGroupProps {
  title: string;
  subtitle: string;
  collapsed?: boolean;
  canRename?: boolean;
  canHide?: boolean;
  isSaving?: boolean;
  errorMessage?: string;
  onRename?: (value: string) => Promise<boolean> | boolean;
  onHide?: () => void;
  onToggle?: () => void;
  children: ComponentChildren;
}

function ChevronIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg
      viewBox="0 0 12 12"
      aria-hidden="true"
      className={`sessionGroupChevron${collapsed ? " isCollapsed" : ""}`}
    >
      <path d="M4 2.5 7.5 6 4 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function SessionGroup({
  title,
  subtitle,
  collapsed = false,
  canRename = false,
  canHide = false,
  isSaving = false,
  errorMessage = "",
  onRename,
  onHide,
  onToggle,
  children,
}: SessionGroupProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState(title);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPosition, setMenuPosition] = useState({ x: 0, y: 0 });
  const savingRef = useRef(false);
  const longPressTimerRef = useRef<number | null>(null);
  const titleTooltip = subtitle?.trim() ? subtitle : title;

  useEffect(() => {
    if (!menuOpen) {
      return;
    }

    const closeMenu = () => setMenuOpen(false);
    window.addEventListener("click", closeMenu);
    window.addEventListener("scroll", closeMenu, true);
    return () => {
      window.removeEventListener("click", closeMenu);
      window.removeEventListener("scroll", closeMenu, true);
    };
  }, [menuOpen]);

  useEffect(() => () => {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current);
    }
  }, []);

  useEffect(() => {
    if (!isEditing) {
      setDraftTitle(title);
    }
  }, [title, isEditing]);

  async function commitRename() {
    if (!onRename || savingRef.current) {
      return;
    }

    savingRef.current = true;
    const saved = await onRename(draftTitle);
    savingRef.current = false;
    if (saved) {
      setIsEditing(false);
    }
  }

  function openMenu(x: number, y: number) {
    if (!canRename && !canHide) {
      return;
    }
    setMenuPosition({ x, y });
    setMenuOpen(true);
  }

  function clearLongPress() {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
  }

  function beginRename() {
    setMenuOpen(false);
    setDraftTitle(title);
    setIsEditing(true);
  }

  function hideGroup() {
    setMenuOpen(false);
    onHide?.();
  }

  return (
    <section className="sessionGroup">
      <div className="sessionGroupShell">
        <div className="sessionGroupHeader" aria-expanded={!collapsed}>
          {isEditing ? (
            <span className="sessionGroupHeading sessionGroupHeadingEditing">
              <input
                type="text"
                className="sessionGroupRenameInput"
                value={draftTitle}
                onInput={(event) => setDraftTitle(event.currentTarget.value)}
                onBlur={() => {
                  void commitRename();
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void commitRename();
                  }
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setDraftTitle(title);
                    setIsEditing(false);
                  }
                }}
                disabled={isSaving}
                autoFocus
              />
            </span>
          ) : (
            <span className="sessionGroupHeading">
              {onToggle ? (
                <button
                  type="button"
                  className="sessionGroupTitleButton"
                  aria-expanded={!collapsed}
                  onClick={onToggle}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    openMenu(event.clientX, event.clientY);
                  }}
                  onPointerDown={(event) => {
                    if (event.pointerType !== "touch") {
                      return;
                    }
                    clearLongPress();
                    longPressTimerRef.current = window.setTimeout(() => {
                      openMenu(event.clientX, event.clientY);
                      longPressTimerRef.current = null;
                    }, 450);
                  }}
                  onPointerUp={clearLongPress}
                  onPointerCancel={clearLongPress}
                  onPointerLeave={clearLongPress}
                  disabled={isSaving}
                  title={titleTooltip}
                >
                  <span className="sessionGroupToggle" aria-hidden="true">
                    <ChevronIcon collapsed={collapsed} />
                  </span>
                  <span className="sessionGroupTitle">{title}</span>
                </button>
              ) : (
                <span className="sessionGroupTitleButton isStatic" title={titleTooltip}>
                  <span className="sessionGroupTitle">{title}</span>
                </span>
              )}
            </span>
          )}
          <span className="sessionGroupActions">
            {canRename ? (
              <button
                type="button"
                className="sessionGroupRenameButton"
                onClick={beginRename}
                disabled={isSaving}
              >
                Rename
              </button>
            ) : null}
          </span>
        </div>
        {errorMessage ? <p className="sessionGroupError">{errorMessage}</p> : null}
        {menuOpen ? (
          <div
            className="sessionGroupMenu"
            style={{ left: `${menuPosition.x}px`, top: `${menuPosition.y}px` }}
            onClick={(event) => event.stopPropagation()}
          >
            {canRename ? (
              <button type="button" className="sessionGroupMenuItem" onClick={beginRename}>
                Rename
              </button>
            ) : null}
            {canHide ? (
              <button type="button" className="sessionGroupMenuItem sessionGroupMenuItemDanger" onClick={hideGroup}>
                Hide working directory
              </button>
            ) : null}
          </div>
        ) : null}
        {collapsed ? null : <div className="sessionGroupList">{children}</div>}
      </div>
    </section>
  );
}
