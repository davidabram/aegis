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
    "role",
    "aria-label",
    "aria-current",
    "aria-selected"
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

    return {
      id,
      tag: node.tagName.toLowerCase(),
      attrs,
      text: readNodeText(node, attrs),
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

  function click(id) {
    const original = findById(id);
    if (!original) {
      throw new Error(`node ${id} not found`);
    }
    const el = resolveActionTarget(original);
    scrollIntoViewIfNeeded(el);
    focusIfPossible(el);
    dispatchPointerLikeEvent(el, "pointerdown");
    dispatchMouseLikeEvent(el, "mousedown");
    dispatchPointerLikeEvent(el, "pointerup");
    dispatchMouseLikeEvent(el, "mouseup");
    if (typeof el.click === "function") {
      el.click();
    } else {
      dispatchMouseLikeEvent(el, "click");
    }
    return { clicked: id, tag: el.tagName.toLowerCase() };
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

  function setValue(id, value) {
    const original = findById(id);
    if (!original) {
      throw new Error(`node ${id} not found`);
    }
    const el = resolveActionTarget(original);
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
    return { id, value };
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
            return { ok: true, value: click(command.id) };
          case "set_value":
            return { ok: true, value: setValue(command.id, command.value) };
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
      const id = assignId(mutation.target);
      if (id != null && mutation.attributeName) {
        changes.push({
          kind: "set_attr",
          id,
          name: mutation.attributeName,
          value: mutation.target.getAttribute(mutation.attributeName)
        });
      }
      return changes;
    }

    if (mutation.type === "characterData") {
      const parent = mutation.target.parentElement;
      const id = assignId(parent);
      if (id != null) {
        changes.push({
          kind: "set_text",
          id,
          text: parent.textContent
        });
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
