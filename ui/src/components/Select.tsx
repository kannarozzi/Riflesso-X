import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { Check, ChevronDown } from "lucide-react";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
}

const LIST_MAX_HEIGHT = 264;
const GAP = 6;

/** A drop-down list in the app's style, keyboard operable like a native
    select (arrows, Home/End, Enter/Space, Escape, typing a letter). The list
    opens upwards when there is not enough room below. */
export function Select<T extends string>({
  id,
  value,
  options,
  onChange,
}: {
  id?: string;
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [placement, setPlacement] = useState({ above: false, maxHeight: LIST_MAX_HEIGHT });
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const listId = useId();
  const selected = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );

  function show() {
    const root = rootRef.current;
    if (!root) return;
    const box = root.getBoundingClientRect();
    const area = scrollArea(root).getBoundingClientRect();
    const below = area.bottom - box.bottom - GAP;
    const above = box.top - area.top - GAP;
    const up = below < LIST_MAX_HEIGHT && above > below;
    setPlacement({ above: up, maxHeight: Math.max(120, Math.min(LIST_MAX_HEIGHT, up ? above : below)) });
    setActive(selected);
    setOpen(true);
  }

  function choose(index: number) {
    onChange(options[index].value);
    setOpen(false);
    buttonRef.current?.focus();
  }

  // Close when clicking anywhere else.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  // Keep the highlighted option in view.
  useEffect(() => {
    if (open) listRef.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  function onKeyDown(e: KeyboardEvent) {
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    const move = (index: number) => {
      e.preventDefault();
      setActive(Math.min(Math.max(index, 0), options.length - 1));
    };
    switch (e.key) {
      case "ArrowDown":
        return move(active + 1);
      case "ArrowUp":
        return move(active - 1);
      case "Home":
        return move(0);
      case "End":
        return move(options.length - 1);
      case "Enter":
      case " ":
        e.preventDefault();
        return choose(active);
      case "Escape":
        e.preventDefault();
        return setOpen(false);
      case "Tab":
        return setOpen(false);
    }
    // Typing a letter jumps to the next option starting with it.
    if (e.key.length === 1) {
      const letter = e.key.toLocaleLowerCase();
      const order = [...options.keys()].map((i) => (active + 1 + i) % options.length);
      const match = order.find((i) => options[i].label.toLocaleLowerCase().startsWith(letter));
      if (match !== undefined) move(match);
    }
  }

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={buttonRef}
        id={id}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listId}
        aria-activedescendant={open ? `${listId}-${active}` : undefined}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
        className={`flex w-full items-center justify-between rounded-xl bg-glass py-2 pr-3 pl-3 text-left text-[13px] outline-none ${
          open
            ? "shadow-[0_0_0_1.5px_var(--color-ink)]"
            : "shadow-[0_0_0_1px_var(--color-edge)] hover:shadow-[0_0_0_1px_var(--color-dim)] focus-visible:shadow-[0_0_0_1.5px_var(--color-ink)]"
        }`}
      >
        <span className="truncate">{options[selected]?.label}</span>
        <ChevronDown className={`size-4 shrink-0 text-dim transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <ul
          ref={listRef}
          id={listId}
          role="listbox"
          style={{
            maxHeight: placement.maxHeight,
            animation: "pop-in 120ms ease-out",
            transformOrigin: placement.above ? "bottom" : "top",
          }}
          className={`absolute inset-x-0 z-20 overflow-y-auto rounded-xl bg-popover p-1 shadow-[0_0_0_1px_var(--color-edge),0_16px_32px_-12px_rgb(0_0_0/0.35)] ${
            placement.above ? "bottom-full mb-1.5" : "top-full mt-1.5"
          }`}
        >
          {options.map((option, i) => {
            const isSelected = i === selected;
            return (
              <li
                key={option.value}
                id={`${listId}-${i}`}
                role="option"
                aria-selected={isSelected}
                onPointerMove={() => setActive(i)}
                onClick={() => choose(i)}
                className={`flex cursor-default items-center justify-between rounded-lg px-2.5 py-2 text-[13px] ${
                  i === active ? "bg-edge" : ""
                } ${isSelected ? "font-semibold" : ""}`}
              >
                <span className="truncate">{option.label}</span>
                {isSelected && <Check className="size-4 shrink-0" />}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/** The nearest ancestor that scrolls (the list must fit inside it), or the window. */
function scrollArea(element: HTMLElement): HTMLElement {
  for (let el = element.parentElement; el; el = el.parentElement) {
    const { overflowY } = getComputedStyle(el);
    if (overflowY === "auto" || overflowY === "scroll") return el;
  }
  return document.documentElement;
}
