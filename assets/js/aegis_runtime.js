(function () {
  if (window.__aegis) {
    return;
  }

  let nextId = 1;
  const queue = [];
  const idByNode = new WeakMap();
  const nodeById = new Map();
  const attrAllowList = new Set([
    "id",
    "class",
    "name",
    "type",
    "value",
    "placeholder",
    "title",
    "href",
    "src",
    "alt",
    "for",
    "autocomplete",
    "disabled",
    "readonly",
    "checked",
    "role",
    "aria-label",
    "aria-current",
    "aria-selected",
    "aria-expanded",
    "aria-pressed",
    "aria-describedby"
  ]);
  const ignoredTags = new Set(["script", "style", "meta", "link", "noscript", "template"]);
  const semanticTextTags = new Set([
    "a",
    "button",
    "label",
    "option",
    "summary",
    "textarea",
    "title",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "li",
    "dt",
    "dd",
    "th",
    "td",
    "legend"
  ]);
  const semanticTextRoles = new Set(["button", "link", "textbox", "searchbox", "option", "tab"]);

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function truncateText(value, maxLength = 200) {
    return value.length > maxLength ? value.slice(0, maxLength) : value;
  }

  function isIgnoredNode(node) {
    return !!node && node.nodeType === Node.ELEMENT_NODE && ignoredTags.has(node.tagName.toLowerCase());
  }

  function assignId(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE || isIgnoredNode(node)) {
      return null;
    }
    let id = idByNode.get(node);
    if (id == null) {
      id = nextId++;
      idByNode.set(node, id);
      nodeById.set(id, node);
    }
    return id;
  }

  function readNodeText(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE || isIgnoredNode(node)) {
      return null;
    }

    const tag = node.tagName.toLowerCase();
    const role = (attrs.role || "").toLowerCase();
    let text = "";

    if (node instanceof HTMLInputElement) {
      text = node.value || attrs.placeholder || attrs["aria-label"] || attrs.title || "";
    } else if (node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) {
      text = node.value || attrs.placeholder || attrs["aria-label"] || attrs.title || "";
    } else if (
      semanticTextTags.has(tag) ||
      semanticTextRoles.has(role) ||
      node.children.length === 0
    ) {
      text = node.innerText || node.textContent || "";
    }

    text = normalizeText(text);
    if (!text) {
      text = normalizeText(
        attrs["aria-label"] || attrs.title || attrs.placeholder || attrs.value || ""
      );
    }
    return text ? truncateText(text) : null;
  }

  function elementValue(node) {
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) {
      return node.value;
    }
    return null;
  }

  function implicitRole(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    const tag = node.tagName.toLowerCase();
    if (attrs.role) {
      return attrs.role;
    }
    if (tag === "a" && attrs.href) return "link";
    if (tag === "button") return "button";
    if (tag === "select") return "combobox";
    if (tag === "textarea") return "textbox";
    if (tag === "option") return "option";
    if (tag === "summary") return "button";
    if (tag === "input") {
      switch ((attrs.type || "text").toLowerCase()) {
        case "search":
          return "searchbox";
        case "button":
        case "submit":
        case "reset":
        case "image":
          return "button";
        case "checkbox":
          return "checkbox";
        case "radio":
          return "radio";
        case "range":
          return "slider";
        default:
          return "textbox";
      }
    }
    return null;
  }

  function associatedLabel(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    if (node instanceof HTMLInputElement ||
        node instanceof HTMLTextAreaElement ||
        node instanceof HTMLSelectElement) {
      if (node.labels && node.labels.length > 0) {
        const text = normalizeText(Array.from(node.labels).map((label) => label.innerText || label.textContent || "").join(" "));
        if (text) return truncateText(text);
      }
    }
    if (node instanceof HTMLLabelElement) {
      const text = normalizeText(node.innerText || node.textContent || "");
      if (text) return truncateText(text);
    }
    return null;
  }

  function accessibleName(node, attrs, text) {
    const label = associatedLabel(node);
    const tag = node && node.nodeType === Node.ELEMENT_NODE ? node.tagName.toLowerCase() : "";
    const name = normalizeText(
      attrs["aria-label"] ||
      label ||
      ((tag === "button" || tag === "input" || tag === "textarea" || tag === "select") ? attrs.title : "") ||
      ((tag === "button" || tag === "input") ? attrs.value : "") ||
      text ||
      attrs.placeholder ||
      attrs.alt ||
      attrs.title ||
      attrs.value ||
      ""
    );
    return name ? truncateText(name) : null;
  }

  function controlType(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }
    const tag = node.tagName.toLowerCase();
    if (tag === "input") {
      const type = (attrs.type || "text").toLowerCase();
      const role = (attrs.role || "").toLowerCase();
      const searchHint = `${attrs["aria-label"] || ""} ${attrs.placeholder || ""} ${attrs.name || ""}`.toLowerCase();
      if (type === "hidden") {
        return "hidden";
      }
      if (type === "search") {
        return "searchbox";
      }
      if (type === "submit" || type === "image") {
        return "submit";
      }
      if (role === "combobox" && searchHint.includes("search")) {
        return "searchbox";
      }
      if (role === "combobox") {
        return "combobox";
      }
      return type;
    }
    if (tag === "button") {
      return (attrs.type || "").toLowerCase() === "submit" ? "submit" : "button";
    }
    if (tag === "textarea" || tag === "select") {
      return tag;
    }
    if (tag === "a" && attrs.href) {
      return "link";
    }
    return null;
  }

  function availableActions(node, attrs) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return [];
    }
    const actions = [];
    const tag = node.tagName.toLowerCase();
    const type = (attrs.type || "").toLowerCase();
    const disabled = node.hasAttribute("disabled") || attrs["aria-disabled"] === "true";
    if (disabled) {
      return actions;
    }
    if (tag === "input" && type === "hidden") {
      return actions;
    }
    if (tag === "a" && attrs.href) {
      actions.push("click", "open");
    }
    if (tag === "button" || (tag === "input" && ["button", "submit", "reset", "checkbox", "radio", "image"].includes(type)) || tag === "summary") {
      actions.push("click");
    }
    if (tag === "input" && ["submit", "image"].includes(type)) {
      actions.push("submit");
    }
    if (tag === "button" && type === "submit") {
      actions.push("submit");
    }
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement || node.isContentEditable) {
      actions.push("type");
    }
    return Array.from(new Set(actions));
  }

  function semanticInfo(node, attrs, text) {
    const role = implicitRole(node, attrs);
    const label = associatedLabel(node);
    const name = accessibleName(node, attrs, text);
    const actions = availableActions(node, attrs);
    const disabled = !!(node && node.nodeType === Node.ELEMENT_NODE &&
      (node.hasAttribute("disabled") || attrs["aria-disabled"] === "true"));
    const control = controlType(node, attrs);
    if (!role && !label && !name && !control && actions.length === 0 && !disabled) {
      return null;
    }
    return {
      role,
      name,
      label,
      control_type: control,
      actionable: control !== "hidden" && actions.length > 0,
      disabled,
      actions
    };
  }

  function serializeNode(node) {
    const id = assignId(node);
    if (id == null) {
      return null;
    }

    const attrs = {};
    for (const attr of Array.from(node.attributes || [])) {
      if (attrAllowList.has(attr.name)) {
        attrs[attr.name] = attr.value;
      }
    }
    const liveValue = elementValue(node);
    if (liveValue != null) {
      attrs.value = liveValue;
    }

    const children = [];
    for (const child of Array.from(node.children || [])) {
      const childId = assignId(child);
      if (childId != null) {
        children.push(childId);
      }
    }

    const text = readNodeText(node, attrs);
    return {
      id,
      tag: node.tagName.toLowerCase(),
      attrs,
      text,
      semantic: semanticInfo(node, attrs, text),
      children
    };
  }

  function collectSerializedSubtree(node, output) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return;
    }
    const serialized = serializeNode(node);
    if (serialized) {
      output.push({ kind: "upsert", ...serialized });
    }
    for (const child of Array.from(node.children || [])) {
      collectSerializedSubtree(child, output);
    }
  }

  function collectRemovedNodeIds(node, output) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return;
    }
    const id = idByNode.get(node);
    if (id != null) {
      output.push({ kind: "remove", id });
      nodeById.delete(id);
    }
    for (const child of Array.from(node.children || [])) {
      collectRemovedNodeIds(child, output);
    }
  }

  function snapshot() {
    if (!document.documentElement) {
      return { nodes: [] };
    }
    const nodes = [];
    const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ELEMENT);
    let current = walker.currentNode;
    while (current) {
      const serialized = isIgnoredNode(current) ? null : serializeNode(current);
      if (serialized) {
        nodes.push(serialized);
      }
      current = walker.nextNode();
    }
    return { nodes };
  }

  function findById(id) {
    return nodeById.get(id) || null;
  }

  function includesNormalized(actual, expected) {
    if (!expected) {
      return true;
    }
    const left = normalizeText(actual || "");
    const right = normalizeText(expected || "");
    if (!right) {
      return true;
    }
    return left.toLowerCase().includes(right.toLowerCase());
  }

  function resolveTarget(target) {
    if (typeof target === "number") {
      const node = findById(target);
      if (!node) {
        throw new Error(`node ${target} not found`);
      }
      return { node, targetId: target, matched: false };
    }

    if (!target || typeof target !== "object") {
      throw new Error("command target must be a node id or matcher object");
    }

    const matcher = target;
    const elements = document.querySelectorAll("*");
    for (const node of elements) {
      const attrs = {};
      for (const attr of Array.from(node.attributes || [])) {
        if (attrAllowList.has(attr.name)) {
          attrs[attr.name] = attr.value;
        }
      }
      const liveValue = elementValue(node);
      if (liveValue != null) {
        attrs.value = liveValue;
      }
      const text = readNodeText(node, attrs);
      const semantic = semanticInfo(node, attrs, text) || {};
      const tag = node.tagName ? node.tagName.toLowerCase() : "";

      if (matcher.role && !includesNormalized(semantic.role, matcher.role)) continue;
      if (matcher.name && !includesNormalized(semantic.name, matcher.name)) continue;
      if (matcher.label && !includesNormalized(semantic.label, matcher.label)) continue;
      if (matcher.control_type && !includesNormalized(semantic.control_type, matcher.control_type)) continue;
      if (matcher.tag && !includesNormalized(tag, matcher.tag)) continue;
      if (matcher.text && !includesNormalized(text, matcher.text)) continue;
      if (matcher.placeholder && !includesNormalized(attrs.placeholder, matcher.placeholder)) continue;
      if (matcher.href_contains && !includesNormalized(attrs.href, matcher.href_contains)) continue;
      if (typeof matcher.actionable === "boolean" && !!semantic.actionable !== matcher.actionable) continue;
      if (typeof matcher.disabled === "boolean" && !!semantic.disabled !== matcher.disabled) continue;

      const targetId = assignId(node);
      if (targetId == null) {
        continue;
      }
      return { node, targetId, matched: true };
    }

    throw new Error(`no node matched ${JSON.stringify(matcher)}`);
  }

  function scrollIntoViewIfNeeded(el) {
    if (typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    }
  }

  function focusIfPossible(el) {
    if (typeof el.focus === "function") {
      el.focus({ preventScroll: true });
    }
  }

  function resolveActionTarget(el) {
    if (el instanceof HTMLLabelElement && el.control) {
      return el.control;
    }
    return el;
  }

  function dispatchMouseLikeEvent(el, type) {
    el.dispatchEvent(
      new MouseEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        view: window,
        button: 0,
        buttons: 1
      })
    );
  }

  function dispatchPointerLikeEvent(el, type) {
    if (typeof PointerEvent !== "function") {
      return;
    }
    el.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        pointerId: 1,
        pointerType: "mouse",
        isPrimary: true,
        button: 0,
        buttons: 1
      })
    );
  }

  function click(target) {
    const resolved = resolveTarget(target);
    const el = resolveActionTarget(resolved.node);
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);
    dispatchPointerLikeEvent(el, "pointerover");
    dispatchMouseLikeEvent(el, "mouseover");
    dispatchPointerLikeEvent(el, "pointermove");
    dispatchMouseLikeEvent(el, "mousemove");
    dispatchPointerLikeEvent(el, "pointerdown");
    dispatchMouseLikeEvent(el, "mousedown");
    dispatchPointerLikeEvent(el, "pointerup");
    dispatchMouseLikeEvent(el, "mouseup");
    const type = el instanceof HTMLInputElement || el instanceof HTMLButtonElement
      ? (el.getAttribute("type") || "").toLowerCase()
      : "";
    if ((el instanceof HTMLButtonElement || el instanceof HTMLInputElement) &&
        (type === "submit" || type === "image") &&
        el.form &&
        typeof el.form.requestSubmit === "function") {
      el.form.requestSubmit(el);
    } else if (typeof el.click === "function") {
      el.click();
    } else {
      dispatchMouseLikeEvent(el, "click");
    }
    return {
      clicked: resolved.targetId,
      matched: resolved.matched,
      tag: el.tagName.toLowerCase(),
      control_type: type || el.tagName.toLowerCase()
    };
  }

  function setNativeElementValue(el, value) {
    const prototype =
      el instanceof HTMLInputElement
        ? HTMLInputElement.prototype
        : el instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : el instanceof HTMLSelectElement
            ? HTMLSelectElement.prototype
            : null;
    const descriptor = prototype && Object.getOwnPropertyDescriptor(prototype, "value");
    if (descriptor && typeof descriptor.set === "function") {
      descriptor.set.call(el, value);
      return;
    }
    el.value = value;
  }

  function setValue(target, value) {
    const resolved = resolveTarget(target);
    const el = resolveActionTarget(resolved.node);
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);
    const previousValue = "value" in el ? el.value : "";

    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
      setNativeElementValue(el, value);
      if (el._valueTracker && typeof el._valueTracker.setValue === "function") {
        el._valueTracker.setValue(previousValue);
      }
    } else if (el.isContentEditable) {
      el.textContent = value;
    } else {
      el.value = value;
    }

    if (typeof InputEvent === "function") {
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          composed: true,
          inputType: "insertText",
          data: String(value)
        })
      );
    } else {
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { id: resolved.targetId, matched: resolved.matched, value };
  }

  function scrollToPosition(x, y) {
    const nextX = Number.isFinite(x) ? x : window.scrollX;
    const nextY = Number.isFinite(y) ? y : window.scrollY;
    window.scrollTo({ left: nextX, top: nextY, behavior: "instant" });
    return { x: window.scrollX, y: window.scrollY };
  }

  function exec(commands) {
    return commands.map((command) => {
      try {
        switch (command.type) {
          case "click":
            return { ok: true, value: click(command.match || command.id) };
          case "set_value":
            return { ok: true, value: setValue(command.match || command.id, command.value) };
          case "scroll":
            return { ok: true, value: scrollToPosition(command.x, command.y) };
          case "eval":
            return { ok: true, value: Function(command.code)() };
          default:
            return { ok: false, error: `unsupported command ${command.type}` };
        }
      } catch (error) {
        return { ok: false, error: String(error && error.message ? error.message : error) };
      }
    });
  }

  function serializeMutation(mutation) {
    const changes = [];

    if (mutation.type === "attributes") {
      if (mutation.target && mutation.target.nodeType === Node.ELEMENT_NODE) {
        const serialized = serializeNode(mutation.target);
        if (serialized) {
          changes.push({ kind: "upsert", ...serialized });
        }
      }
      return changes;
    }

    if (mutation.type === "characterData") {
      const parent = mutation.target.parentElement;
      if (parent) {
        const serialized = serializeNode(parent);
        if (serialized) {
          changes.push({ kind: "upsert", ...serialized });
        }
      }
      return changes;
    }

    if (mutation.type === "childList") {
      for (const node of Array.from(mutation.addedNodes || [])) {
        collectSerializedSubtree(node, changes);
      }
      for (const node of Array.from(mutation.removedNodes || [])) {
        collectRemovedNodeIds(node, changes);
      }

      const target =
        mutation.target && mutation.target.nodeType === Node.ELEMENT_NODE
          ? mutation.target
          : mutation.target && mutation.target.parentElement
            ? mutation.target.parentElement
            : null;
      const id = assignId(target);
      if (id != null) {
        changes.push({
          kind: "set_children",
          id,
          children: Array.from(target.children || [])
            .map((child) => assignId(child))
            .filter((childId) => childId != null)
        });
      }
    }

    return changes;
  }

  function drainEvents() {
    return queue.splice(0, queue.length);
  }

  const observer = new MutationObserver((mutations) => {
    const changes = mutations.flatMap(serializeMutation);
    if (changes.length > 0) {
      queue.push({
        event: {
          type: "dom_mutation",
          changes
        }
      });
    }
  });

  let observerAttached = false;

  function attachObserver() {
    if (observerAttached || !document.documentElement) {
      return;
    }
    observerAttached = true;
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true
    });
  }

  attachObserver();
  if (!observerAttached) {
    document.addEventListener("DOMContentLoaded", attachObserver, { once: true });
  }

  window.__aegis_queue = queue;
  window.__aegis = {
    snapshot,
    exec,
    click,
    setValue,
    scrollToPosition,
    drainEvents,
    queue,
    assignId
  };
})();
