import { test } from "node:test";
import assert from "node:assert/strict";
import { selectPosition } from "../src/components/common/selectPosition.ts";
import { getDraft, setDraft, promoteDraft, getActiveConversation, setActiveConversation, subscribeDrafts, NEW_CONVERSATION } from "../src/agent/drafts.ts";

const document = text => ({ type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text }] }] });

test("model menus open upward only when the lower space is insufficient", () => {
  const bounds = { left: 0, top: 0, right: 780, bottom: 480 };
  const up = selectPosition({ left: 650, right: 750, top: 440, bottom: 466 }, bounds, 130, 240);
  assert.equal(up.upwards, true);
  assert.equal(up.left, 540);
  assert.ok(up.top + up.maxHeight < 440);
  const down = selectPosition({ left: 20, right: 180, top: 10, bottom: 36 }, bounds, 130, 240);
  assert.equal(down.upwards, false);
  assert.equal(down.top, 42);
  const small = selectPosition({ left: 0, right: 130, top: 80, bottom: 100 }, { left: 0, top: 0, right: 140, bottom: 150 }, 260, 240);
  assert.equal(small.width, 140);
  assert.ok(small.maxHeight <= 74);
});

test("session drafts remain independent and retain rich plugin mentions", () => {
  let notifications = 0;
  const stop = subscribeDrafts(() => notifications++);
  const rich = { type: "doc", content: [{ type: "paragraph", content: [{ type: "mention", attrs: { id: "netease", label: "网易云音乐" } }, { type: "text", text: " 查询状态" }] }] };
  setDraft("a",rich); setDraft("b",document("另一段草稿")); setDraft(NEW_CONVERSATION,document("新对话草稿"));
  setActiveConversation("b"); setActiveConversation("a");
  assert.deepEqual(getDraft("a"),rich);
  assert.equal(getDraft("b").content[0].content[0].text,"另一段草稿");
  setActiveConversation(NEW_CONVERSATION);
  assert.equal(getDraft(NEW_CONVERSATION).content[0].content[0].text,"新对话草稿");
  setDraft("a",{type:"doc",content:[{type:"paragraph"}]});
  assert.equal(getDraft("b").content[0].content[0].text,"另一段草稿");
  assert.ok(notifications>=6); stop();
});

test("promoting a new conversation moves only its draft and active identity", () => {
  setDraft(NEW_CONVERSATION,document("创建过程中继续写的草稿"));
  promoteDraft("materialized-session");
  assert.equal(getActiveConversation(),"materialized-session");
  assert.equal(getDraft("materialized-session").content[0].content[0].text,"创建过程中继续写的草稿");
  assert.deepEqual(getDraft(NEW_CONVERSATION),{type:"doc",content:[{type:"paragraph"}]});
  assert.equal(getDraft("b").content[0].content[0].text,"另一段草稿");
});

test("drafts survive a module reload through browser local storage", async () => {
  const values = new Map();
  globalThis.localStorage = { getItem: key=>values.get(key)||null, setItem: (key,value)=>values.set(key,value) };
  setDraft("persistent",document("恢复草稿")); setActiveConversation("persistent");
  const restored = await import(`../src/agent/drafts.ts?reload=${Date.now()}`);
  assert.equal(restored.getActiveConversation(),"persistent");
  assert.equal(restored.getDraft("persistent").content[0].content[0].text,"恢复草稿");
  delete globalThis.localStorage;
});
