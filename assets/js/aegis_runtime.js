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
    "href",
    "src",
    "role",
    "aria-label"
  ]);

  function assignId(node) {
    if (!node || node.nodeType !== Node.ELEMENT_NODE) {
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
      text: node.children.length === 0 ? node.textContent : null,
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
    const nodes = [];
    const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ELEMENT);
    let current = walker.currentNode;
    while (current) {
      const serialized = serializeNode(current);
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

  function click(id) {
    const el = findById(id);
    if (!el) {
      throw new Error(`node ${id} not found`);
    }
    if (typeof el.click === "function") {
      el.click();
      return { clicked: id };
    }
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    return { clicked: id };
  }

  function setValue(id, value) {
    const el = findById(id);
    if (!el) {
      throw new Error(`node ${id} not found`);
    }
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
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

  if (document.documentElement) {
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true
    });
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
