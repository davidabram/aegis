#!/usr/bin/env python3

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: generate_runtime_header.py <input_js> <output_header>", file=sys.stderr)
        return 1

    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])
    source = input_path.read_text()
    escaped = json.dumps(source)

    output = "\n".join(
        [
            "#ifndef AEGIS_NATIVE_AEGIS_RUNTIME_SCRIPT_H_",
            "#define AEGIS_NATIVE_AEGIS_RUNTIME_SCRIPT_H_",
            "",
            f"inline constexpr char kAegisRuntimeScript[] = {escaped};",
            "",
            "#endif",
            "",
        ]
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
