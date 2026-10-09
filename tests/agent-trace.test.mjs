import { test } from "node:test";
import assert from "node:assert/strict";
import { emptyTrace, reduceTrace } from "../src/agent/trace.ts";
import { legacyEntries } from "../src/widgets/ai/legacy.ts";

const event = (seq, eventType, payload = {}, blockId = null, callId = null) => ({ version: 1, id: `event-${seq}`, sessionId: "session", runId: "run", seq, eventType, payload, blockId, callId, createdAt: "0" });
const script = [
  event(1, "run_started"), event(2, "user_message", { text: "播放什么" }),
  event(3, "block_started", { kind: "reasoning" }, "r"), event(4, "reasoning_delta", { text: "先查询" }, "r"), event(5, "block_ended", {}, "r"),
  event(6, "block_started", { kind: "assistant" }, "a"), event(7, "text_delta", { text: "正在查询。" }, "a"),
  event(8, "block_started", { kind: "tool" }, "t"), event(9, "tool_delta", { name: "netease.get_current_state", title: "读取播放状态", argumentsDelta: "{" }, "t", "call-1"),
  event(10, "tool_delta", { argumentsDelta: "}" }, "t"), event(11, "tool_started", { arguments: "{}" }, "t", "call-1"),
  event(12, "tool_completed", { output: { title: "歌曲" }, summary: "成功", status: "completed" }, "t", "call-1"),
  event(13, "block_started", { kind: "assistant" }, "b"), event(14, "text_delta", { text: "当前歌曲。" }, "b"), event(15, "run_completed"),
];

test("live projection equals replay and retains text/tool/text order", () => {
  let state = emptyTrace();
  for (const e of script) state = reduceTrace(state, [e]);
  assert.deepEqual(state.entries, reduceTrace(emptyTrace(), script).entries);
  assert.deepEqual(state.entries.map(e => e.kind), ["user", "reasoning", "assistant", "tool", "assistant"]);
  assert.equal(state.entries[3].callId, "call-1"); assert.equal(state.entries[3].arguments, "{}"); assert.equal(state.entries[3].status, "completed");
  assert.equal(reduceTrace(state, script).entries[2].text, "正在查询。");
});

test("missing events are buffered until replay fills the gap", () => {
  let state = reduceTrace(emptyTrace(), [script[1], script[3]]);
  assert.equal(state.lastSeq, 0); assert.equal(state.entries.length, 0);
  state = reduceTrace(state, [script[0], script[2]]);
  assert.equal(state.lastSeq, 4); assert.equal(state.entries[1].detail, "先查询");
});

test("tool failure survives a successful final answer", () => {
  const state = reduceTrace(emptyTrace(), script.map(e => e.eventType === "tool_completed" ? { ...e, payload: { ...e.payload, status: "failed" } } : e));
  assert.equal(state.entries[3].status, "failed"); assert.equal(state.entries[4].status, "completed");
});

test("interrupted and failed runs keep partial text and actionable diagnostics", () => {
  const partial = script.slice(0, 11);
  for (const eventType of ["run_failed", "run_cancelled", "run_interrupted"]) {
    const state = reduceTrace(emptyTrace(), [...partial, event(12, eventType, { message: "HTTP 400: bad name", error: { category: "INVALID_REQUEST", status: 400, param: "tools.0.name" } })]);
    assert.equal(state.entries[2].text, "正在查询。"); assert.equal(state.entries.at(-1).kind, "error");
    assert.equal(state.entries.at(-1).error.param, "tools.0.name"); assert.notEqual(state.entries[3].status, "running");
  }
});

test("successive runs keep distinct blocks and remove only their optimistic user", () => {
  const second = script.map(e => ({ ...e, id: `second-${e.id}`, seq: e.seq + script.length, runId: "second", blockId: e.blockId ? `second-${e.blockId}` : null }));
  let state = reduceTrace(emptyTrace(), script);
  state = { ...state, entries: [...state.entries, { kind: "user", id: "local-user-second", runId: "second", text: "optimistic", position: 16 }] };
  state = reduceTrace(state, [...second].reverse());
  assert.equal(state.entries.length, 10);
  assert.equal(state.entries.filter(e => e.kind === "user").length, 2);
  assert.equal(state.entries[7].text, "正在查询。");
  assert.equal(state.lastSeq, 30);
  assert.deepEqual([...state.terminalRuns], ["run", "second"]);
  assert.deepEqual(reduceTrace(state, second).entries, state.entries);
});

test("legacy plain messages remain readable without fabricated tool calls", () => {
  const legacy = legacyEntries([], [{ id: "old", role: "assistant", content: "old answer", createdAt: "0" }]);
  const state = reduceTrace(emptyTrace(legacy), script);
  assert.equal(state.entries[0].text, "old answer");
  assert.equal(state.entries.filter(e => e.kind === "tool").length, 1);
  assert.ok(state.entries[0].position < state.entries[1].position);
});
