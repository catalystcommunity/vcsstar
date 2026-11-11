// Native DOM building. There is no virtual DOM and no signal graph: a page
// builds its elements one time, keeps references to the parts that change,
// and changes only those parts (the Zest rule).

export type Child = Node | string | number | false | null | undefined | Child[];

type Handler = (event: any) => void;

export interface Props {
  class?: string;
  /** Event listeners, by event name. */
  on?: Record<string, Handler>;
  /** Properties to assign, for values that are not text (for example a
   * combo box's `options`). */
  props?: Record<string, unknown>;
  ref?: (element: any) => void;
  /** Any other key is an attribute. `true` gives an empty attribute;
   * `false`, `null`, and `undefined` leave it out. */
  [attribute: string]: unknown;
}

/** The DOM nodes for `children`, with text for strings and numbers. */
export function nodes(children: Child): Node[] {
  if (Array.isArray(children)) return children.flatMap(nodes);
  if (children === null || children === undefined || children === false) return [];
  if (children instanceof Node) return [children];
  return [document.createTextNode(String(children))];
}

/** Make an element with attributes, listeners, properties, and children. */
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props?: Props | null, ...children: Child[]): HTMLElementTagNameMap[K];
export function h(tag: string, props?: Props | null, ...children: Child[]): HTMLElement;
export function h(tag: string, props: Props | null = null, ...children: Child[]): HTMLElement {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(props ?? {})) {
    if (name === "on") for (const [event, handler] of Object.entries((value ?? {}) as Record<string, Handler>)) element.addEventListener(event, handler);
    else if (name === "props") Object.assign(element, value ?? {});
    else if (name === "ref") continue;
    else attribute(element, name, value);
  }
  element.append(...nodes(children));
  props?.ref?.(element);
  return element;
}

/** Set, or remove, one attribute. */
export function attribute(element: Element, name: string, value: unknown): void {
  if (value === false || value === null || value === undefined) element.removeAttribute(name);
  else element.setAttribute(name, value === true ? "" : String(value));
}

/** Replace the children of `element`. */
export function fill(element: Element, ...children: Child[]): void {
  element.replaceChildren(...nodes(children));
}

/** Show or hide an element. */
export function show(element: HTMLElement, visible: unknown): void {
  element.hidden = !visible;
}

/** A link that the app routes without a page load. */
export function link(href: string, ...children: Child[]): HTMLAnchorElement {
  return h("a", { href, "data-nav": true }, ...children);
}

/** A link to another site, in a new tab. */
export function external(href: string, ...children: Child[]): HTMLAnchorElement {
  return h("a", { href, target: "_blank", rel: "external noreferrer" }, ...children);
}

// ---------------------------------------------------------------------------
// Controls. Each helper makes a Zest control with its common attributes, so
// that the pages read like the forms they are.
// ---------------------------------------------------------------------------

export interface FieldOptions {
  name?: string;
  type?: string;
  value?: string;
  required?: boolean;
  autocomplete?: string;
  placeholder?: string;
  helper?: string;
  maxlength?: number;
  readonly?: boolean;
  inputmode?: string;
  autocapitalize?: string;
  spellcheck?: boolean;
  autofocus?: boolean;
  "aria-label"?: string;
  class?: string;
  on?: Record<string, Handler>;
}

/** A Zest text field. `label` is the visible label; pass `aria-label`
 * instead for a field with no visible label. */
export function textField(label: string, options: FieldOptions = {}): ZestTextField {
  const { on, value, spellcheck, ...attributes } = options;
  const field = h("text-field", { label: label || undefined, ...attributes, spellcheck: spellcheck === false ? "false" : undefined, on }) as ZestTextField;
  if (value !== undefined) field.setAttribute("value", value);
  return field;
}

export function textArea(label: string, options: FieldOptions & { rows?: number; autogrow?: boolean } = {}): ZestTextArea {
  const { on, value, ...attributes } = options;
  const area = h("text-area", { label: label || undefined, ...attributes, on }) as ZestTextArea;
  if (value !== undefined) area.setAttribute("value", value);
  return area;
}

export function checkBox(label: Child, options: { checked?: boolean; disabled?: boolean; name?: string; class?: string; on?: Record<string, Handler> } = {}): ZestCheckBox {
  return h("check-box", { ...options }, label) as ZestCheckBox;
}

/** A Zest combo box. Set `options` first, then the value. */
export function comboBox(label: string, options: ZestOption[], settings: { value?: string; editable?: boolean; required?: boolean; disabled?: boolean; placeholder?: string; name?: string; class?: string; "aria-label"?: string; on?: Record<string, Handler> } = {}): ZestComboBox {
  const { value, on, ...attributes } = settings;
  const box = h("combo-box", { label: label || undefined, ...attributes, on, props: { options } }) as ZestComboBox;
  if (value !== undefined) box.setAttribute("value", value);
  return box;
}

export function spinBox(label: string, settings: { value?: number; min?: number; max?: number; step?: number; class?: string; on?: Record<string, Handler> } = {}): ZestSpinBox {
  return h("spin-box", { label, ...settings }) as ZestSpinBox;
}

export interface ButtonOptions {
  type?: "button" | "submit";
  variant?: "default" | "primary" | "danger" | "flat";
  disabled?: boolean;
  "aria-label"?: string;
  class?: string;
  title?: string;
  on?: Record<string, Handler>;
  onClick?: Handler;
}

export function button(text: Child, options: ButtonOptions = {}): ZestPushButton {
  const { onClick, on, type = "button", variant = type === "submit" ? "primary" : undefined, ...attributes } = options;
  return h("push-button", { type, variant, ...attributes, on: { ...on, ...(onClick ? { click: onClick } : {}) } }, text) as ZestPushButton;
}

/** A quiet button for a secondary or a row action. */
export function textButton(text: Child, onClick: Handler, options: { disabled?: boolean; "aria-label"?: string; danger?: boolean; title?: string } = {}): ZestPushButton {
  const { danger, ...rest } = options;
  return button(text, { variant: "flat", class: danger ? "text-button danger" : "text-button", onClick, ...rest });
}

/** Empty fields after a successful submit, without the "fill this out"
 * problem that a touched empty field would show. */
export function clear(...fields: Array<{ value: string; touched?: boolean; invalidate?: () => void }>): void {
  for (const field of fields) {
    field.value = "";
    if ("touched" in field) field.touched = false;
    field.invalidate?.();
  }
}

/** Turn a group of controls on or off together. */
export function setDisabled(controls: Iterable<{ disabled: boolean }>, disabled: boolean): void {
  for (const control of controls) control.disabled = disabled;
}

/** A form that calls `submit` with the native submit event prevented. Each
 * Zest control in it is form associated, so the browser checks `required`
 * before the event, and Enter in a `text-field` submits the form. */
export function form(props: Props & { onSubmit: (event: SubmitEvent) => void }, ...children: Child[]): HTMLFormElement {
  const { onSubmit, on, ...rest } = props;
  return h("form", { ...rest, on: { ...on, submit: (event: SubmitEvent) => { event.preventDefault(); onSubmit(event); } } }, ...children);
}

/** Mark a form busy: `aria-busy`, and the given controls off. */
export function setBusy(element: HTMLElement, busy: boolean, controls: Iterable<{ disabled: boolean }> = []): void {
  attribute(element, "aria-busy", busy ? "true" : null);
  setDisabled(controls, busy);
}
