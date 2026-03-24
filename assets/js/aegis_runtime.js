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
    "aria-describedby",
    "aria-disabled"
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
  const semanticTextRoles = new Set(["button", "link", "textbox", "searchbox", "option", "tab", "combobox"]);

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function truncateText(value, maxLength = 200) {
    return value.length > maxLength ? value.slice(0, maxLength) : value;
  }

  function currentPageState() {
    return {
      url: window.location ? window.location.href : null,
      title: document.title || null,
      ready_state: document.readyState || null
    };
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

  function isElementVisible(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
      return false;
    }
    const style = window.getComputedStyle ? window.getComputedStyle(node) : null;
    if (node.hasAttribute("hidden")) {
      return false;
    }
    if (style && (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse")) {
      return false;
    }
    if (node.getAttribute("aria-hidden") === "true") {
      return false;
    }
    const rect = typeof node.getBoundingClientRect === "function" ? node.getBoundingClientRect() : null;
    if (!rect) {
      return true;
    }
    return rect.width > 0 && rect.height > 0;
  }

  function elementValue(node) {
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) {
      return node.value;
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
        if (text) {
          return truncateText(text);
        }
      }
    }
    if (node instanceof HTMLLabelElement) {
      const text = normalizeText(node.innerText || node.textContent || "");
      if (text) {
        return truncateText(text);
      }
    }
    return null;
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
    if (node.isContentEditable) return "textbox";
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
    if (node.isContentEditable) {
      return "textbox";
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
      actions.push("click", "open", "hover", "press_key");
    }
    if (tag === "button" || (tag === "input" && ["button", "submit", "reset", "checkbox", "radio", "image"].includes(type)) || tag === "summary") {
      actions.push("click", "hover", "press_key");
    }
    if (tag === "input" && [ "submit", "image" ].includes(type)) {
      actions.push("submit");
    }
    if (tag === "button" && type === "submit") {
      actions.push("submit");
    }
    if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement || node.isContentEditable) {
      actions.push("type", "focus", "hover", "press_key");
    }
    if (tag === "label" || attrs.role || tag === "div" || tag === "span") {
      actions.push("hover");
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
    const visible = isElementVisible(node);
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
      visible,
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

  function includesNormalized(actual, expected, exactOnly) {
    const left = normalizeText(actual || "").toLowerCase();
    const right = normalizeText(expected || "").toLowerCase();
    if (!right) {
      return true;
    }
    if (!left) {
      return false;
    }
    return exactOnly ? left === right : left.includes(right);
  }

  function describeNode(node) {
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
    const semantic = semanticInfo(node, attrs, text) || {
      role: null,
      name: null,
      label: null,
      control_type: null,
      actionable: false,
      disabled: false,
      visible: false,
      actions: []
    };
    return {
      node,
      id: assignId(node),
      tag: node.tagName ? node.tagName.toLowerCase() : "",
      attrs,
      text,
      semantic
    };
  }

  function placeholderHaystack(descriptor) {
    return [
      descriptor.attrs.placeholder || "",
      descriptor.attrs["aria-label"] || "",
      descriptor.semantic.name || "",
      descriptor.semantic.label || ""
    ].join(" ");
  }

  function isTargetableForAction(descriptor, action) {
    if (!descriptor.semantic.visible || descriptor.semantic.disabled) {
      return false;
    }
    if (action === "click") {
      return descriptor.semantic.actions.includes("click") ||
        descriptor.semantic.actions.includes("open") ||
        descriptor.semantic.actions.includes("submit") ||
        descriptor.tag === "label";
    }
    if (action === "type") {
      return descriptor.semantic.actions.includes("type");
    }
    if (action === "hover") {
      return descriptor.semantic.actions.includes("hover") || descriptor.semantic.visible;
    }
    if (action === "press_key") {
      return descriptor.semantic.actions.includes("press_key") ||
        descriptor.semantic.actions.includes("focus") ||
        descriptor.semantic.actions.includes("type");
    }
    return true;
  }

  function scoreStringField(field, actual, expected, exactOnly, fields) {
    if (!expected) {
      return 0;
    }
    const left = normalizeText(actual || "");
    const right = normalizeText(expected || "");
    if (!left || !right) {
      return null;
    }
    if (left.toLowerCase() === right.toLowerCase()) {
      fields.push({ field, exact: true, score: 120 });
      return 120;
    }
    if (exactOnly || !left.toLowerCase().includes(right.toLowerCase())) {
      return null;
    }
    fields.push({ field, exact: false, score: 60 });
    return 60;
  }

  function scoreCandidate(descriptor, matcher, action) {
    if (typeof matcher.actionable === "boolean" && !!descriptor.semantic.actionable !== matcher.actionable) {
      return null;
    }
    if (typeof matcher.disabled === "boolean" && !!descriptor.semantic.disabled !== matcher.disabled) {
      return null;
    }

    if (action && !isTargetableForAction(descriptor, action)) {
      return null;
    }

    const fields = [];
    const exactOnly = !!matcher.exact;
    let score = 0;
    const checks = [
      ["role", descriptor.semantic.role, matcher.role],
      ["name", descriptor.semantic.name, matcher.name],
      ["label", descriptor.semantic.label, matcher.label],
      ["control_type", descriptor.semantic.control_type, matcher.control_type],
      ["tag", descriptor.tag, matcher.tag],
      ["text", descriptor.text, matcher.text],
      ["placeholder", placeholderHaystack(descriptor), matcher.placeholder],
      ["href_contains", descriptor.attrs.href, matcher.href_contains]
    ];

    for (const [field, actual, expected] of checks) {
      const fieldScore = scoreStringField(field, actual, expected, exactOnly, fields);
      if (fieldScore == null) {
        return null;
      }
      score += fieldScore;
    }

    if (descriptor.semantic.visible) {
      score += 40;
    }
    if (descriptor.semantic.actionable) {
      score += 20;
    }
    if (action && isTargetableForAction(descriptor, action)) {
      score += 120;
    }
    if (descriptor.tag === "a" || descriptor.tag === "button" || descriptor.tag === "input" || descriptor.tag === "textarea" || descriptor.tag === "select") {
      score += 10;
    }

    return {
      descriptor,
      score,
      fields
    };
  }

  function resolutionDebug(candidates) {
    return candidates.slice(0, 3).map((candidate) => ({
      id: candidate.descriptor.id,
      tag: candidate.descriptor.tag,
      text: candidate.descriptor.text,
      role: candidate.descriptor.semantic.role || null,
      name: candidate.descriptor.semantic.name || null,
      control_type: candidate.descriptor.semantic.control_type || null,
      actionable: !!candidate.descriptor.semantic.actionable,
      visible: !!candidate.descriptor.semantic.visible,
      score: candidate.score,
      fields: candidate.fields
    }));
  }

  function resolveTarget(target, action) {
    if (typeof target === "number") {
      const node = findById(target);
      if (!node) {
        throw new Error(`node ${target} not found`);
      }
      const descriptor = describeNode(node);
      if (action && !isTargetableForAction(descriptor, action)) {
        throw new Error(`node ${target} is not targetable for ${action}`);
      }
      return {
        node,
        targetId: target,
        matched: false,
        debug: {
          candidate_count: 1,
          chosen: resolutionDebug([{ descriptor, score: 0, fields: [] }])[0],
          candidates: resolutionDebug([{ descriptor, score: 0, fields: [] }])
        }
      };
    }

    if (!target || typeof target !== "object") {
      throw new Error("command target must be a node id or matcher object");
    }

    const matcher = target;
    const candidates = [];
    for (const node of document.querySelectorAll("*")) {
      if (isIgnoredNode(node)) {
        continue;
      }
      const descriptor = describeNode(node);
      if (descriptor.id == null) {
        continue;
      }
      const candidate = scoreCandidate(descriptor, matcher, action);
      if (candidate) {
        candidates.push(candidate);
      }
    }

    candidates.sort((left, right) => {
      if (right.score !== left.score) {
        return right.score - left.score;
      }
      if (!!right.descriptor.semantic.actionable !== !!left.descriptor.semantic.actionable) {
        return right.descriptor.semantic.actionable ? 1 : -1;
      }
      return left.descriptor.id - right.descriptor.id;
    });

    if (candidates.length === 0) {
      throw new Error(`no node matched ${JSON.stringify(matcher)}`);
    }

    if (candidates.length > 1 && candidates[0].score === candidates[1].score) {
      throw new Error(`ambiguous node match ${JSON.stringify({
        matcher,
        candidates: resolutionDebug(candidates)
      })}`);
    }

    return {
      node: candidates[0].descriptor.node,
      targetId: candidates[0].descriptor.id,
      matched: true,
      debug: {
        candidate_count: candidates.length,
        chosen: resolutionDebug(candidates)[0],
        candidates: resolutionDebug(candidates)
      }
    };
  }

  function scrollIntoViewIfNeeded(el) {
    if (typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    }
  }

  function focusIfPossible(el) {
    if (typeof el.focus === "function") {
      try {
        el.focus({ preventScroll: true });
      } catch (_error) {
        el.focus();
      }
    }
  }

  function resolveActionTarget(el, action) {
    if ((action === "click" || action === "type") && el instanceof HTMLLabelElement && el.control) {
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

  function baseActionResult(resolved, el, before, after) {
    const type = el instanceof HTMLInputElement || el instanceof HTMLButtonElement
      ? (el.getAttribute("type") || "").toLowerCase()
      : "";
    return {
      matched: resolved.matched,
      tag: el.tagName.toLowerCase(),
      control_type: type || el.tagName.toLowerCase(),
      actionable: true,
      target: resolved.debug.chosen,
      matcher_debug: resolved.debug,
      page_before: before,
      page_after: after,
      navigation_changed: before.url !== after.url,
      title_changed: before.title !== after.title
    };
  }

  function click(target) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "click");
    const el = resolveActionTarget(resolved.node, "click");
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
    const after = currentPageState();
    return {
      clicked: resolved.targetId,
      ...baseActionResult(resolved, el, before, after)
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
    const before = currentPageState();
    const resolved = resolveTarget(target, "type");
    const el = resolveActionTarget(resolved.node, "type");
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
    } else if ("value" in el) {
      el.value = value;
    } else {
      throw new Error(`node ${resolved.targetId} is not typeable`);
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
    const after = currentPageState();
    return {
      id: resolved.targetId,
      value,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function hover(target) {
    const before = currentPageState();
    const resolved = resolveTarget(target, "hover");
    const el = resolveActionTarget(resolved.node, "hover");
    scrollIntoViewIfNeeded(el);
    dispatchPointerLikeEvent(el, "pointerover");
    dispatchMouseLikeEvent(el, "mouseover");
    dispatchPointerLikeEvent(el, "pointerenter");
    dispatchMouseLikeEvent(el, "mouseenter");
    dispatchPointerLikeEvent(el, "pointermove");
    dispatchMouseLikeEvent(el, "mousemove");
    const after = currentPageState();
    return {
      hovered: resolved.targetId,
      ...baseActionResult(resolved, el, before, after)
    };
  }

  function dispatchKeyEvent(el, type, options) {
    const event = new KeyboardEvent(type, {
      bubbles: true,
      cancelable: true,
      composed: true,
      key: options.key,
      code: options.code || options.key,
      altKey: !!options.altKey,
      ctrlKey: !!options.ctrlKey,
      metaKey: !!options.metaKey,
      shiftKey: !!options.shiftKey
    });
    return el.dispatchEvent(event);
  }

  function pressKey(target, key, options) {
    const before = currentPageState();
    let resolved = null;
    let el = document.activeElement || document.body;
    if (target != null) {
      resolved = resolveTarget(target, "press_key");
      el = resolveActionTarget(resolved.node, "press_key");
      scrollIntoViewIfNeeded(el);
      focusIfPossible(el);
    } else {
      focusIfPossible(el);
    }

    const eventOptions = {
      key,
      code: options && options.code ? options.code : key,
      altKey: !!(options && options.altKey),
      ctrlKey: !!(options && options.ctrlKey),
      metaKey: !!(options && options.metaKey),
      shiftKey: !!(options && options.shiftKey)
    };

    const keydownAccepted = dispatchKeyEvent(el, "keydown", eventOptions);
    const keypressAccepted = dispatchKeyEvent(el, "keypress", eventOptions);
    let triggeredSubmit = false;
    if (keydownAccepted && keypressAccepted && key === "Enter") {
      if (el.form && typeof el.form.requestSubmit === "function") {
        el.form.requestSubmit();
        triggeredSubmit = true;
      } else if ((el instanceof HTMLButtonElement || el instanceof HTMLAnchorElement) && typeof el.click === "function") {
        el.click();
      }
    }
    dispatchKeyEvent(el, "keyup", eventOptions);
    const after = currentPageState();
    return {
      key,
      code: eventOptions.code,
      triggered_submit: triggeredSubmit,
      target: resolved ? resolved.debug.chosen : null,
      matcher_debug: resolved ? resolved.debug : null,
      page_before: before,
      page_after: after,
      navigation_changed: before.url !== after.url,
      title_changed: before.title !== after.title
    };
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
          case "hover":
            return { ok: true, value: hover(command.match || command.id) };
          case "set_value":
            return { ok: true, value: setValue(command.match || command.id, command.value) };
          case "press_key":
            return {
              ok: true,
              value: pressKey(
                command.target ? (command.target.match || command.target.id) : null,
                command.key,
                {
                  code: command.code,
                  altKey: command.alt_key,
                  ctrlKey: command.ctrl_key,
                  metaKey: command.meta_key,
                  shiftKey: command.shift_key
                }
              )
            };
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
    hover,
    pressKey,
    setValue,
    scrollToPosition,
    drainEvents,
    queue,
    assignId,
    currentPageState
  };
})();
