import { api } from "./api";

/** Small card offered for a few seconds after a paste. */
export function buildFixCard(): HTMLElement {
  const card = el("div", "fix-card");
  card.hidden = true;
  const first = document.createElement("button");
  first.type = "button";
  first.className = "fix-main";
  first.textContent = "↻ Fix last";
  const choices = el("div", "fix-choices");
  choices.hidden = true;
  const again = document.createElement("button");
  again.type = "button";
  again.textContent = "Listen again";
  const type = document.createElement("button");
  type.type = "button";
  type.textContent = "Type it";
  choices.append(again, type);
  const close = document.createElement("button");
  close.type = "button";
  close.className = "fix-close";
  close.textContent = "✕";
  close.setAttribute("aria-label", "No thanks");
  first.addEventListener("click", () => {
    first.hidden = true;
    choices.hidden = false;
  });
  const send = (action: "relisten" | "type" | "dismiss") => {
    void api.overlayFix(action).catch((error) => console.warn("fix", error));
  };
  again.addEventListener("click", () => send("relisten"));
  type.addEventListener("click", () => send("type"));
  close.addEventListener("click", () => send("dismiss"));
  card.append(first, choices, close);
  return card;
}

export function resetFixCard(card: HTMLElement): void {
  const first = card.querySelector<HTMLElement>(".fix-main");
  const choices = card.querySelector<HTMLElement>(".fix-choices");
  if (first) first.hidden = false;
  if (choices) choices.hidden = true;
}

function el(tag: string, className?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}
