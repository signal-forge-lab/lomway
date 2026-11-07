"""Read-only smoke test for the Browser Harness stdio-to-HTTP MCP bridge."""
import argparse
import asyncio
import json
from mcp import ClientSession
from mcp.client.streamable_http import streamable_http_client

EXPECTED = {"browser_page_info", "browser_list_tabs", "browser_new_tab", "browser_js"}

async def smoke(url: str, browser_check: bool) -> None:
    async with streamable_http_client(url) as streams:
        read, write = streams[:2]
        async with ClientSession(read, write) as session:
            result = await session.initialize()
            tools = await session.list_tools()
            names = {t.name for t in tools.tools}
            missing = sorted(EXPECTED - names)
            if missing:
                raise RuntimeError(f"missing MCP tools: {missing}")
            report = {"protocol": str(result.protocol_version), "tools": len(names), "required_present": True}
            if browser_check:
                page = await session.call_tool("browser_list_tabs", {})
                report["browser_call_is_error"] = bool(page.isError)
                report["browser_result"] = [c.text[:500] for c in page.content if hasattr(c, "text")][:2]
            print(json.dumps(report, ensure_ascii=False))

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--url", default="http://127.0.0.1:17685/mcp")
    p.add_argument("--browser-check", action="store_true")
    a = p.parse_args()
    asyncio.run(smoke(a.url, a.browser_check))
