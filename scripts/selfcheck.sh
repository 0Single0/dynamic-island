#!/bin/sh
# 部署自检：对着一个已经跑起来的面板 + 下游面，验证关键链路。
#
#   sh scripts/selfcheck.sh http://<面板地址:8791> http://<下游地址:8790> [用户名] [密码]
#
# 只做只读探测与一次登录，不会改任何配置（除了下面明确说明的 API Key 一节）。
set -e

PANEL="${1:?用法: selfcheck.sh <panel_url> <gateway_url> [user] [pass]}"
DOWN="${2:?缺少下游地址}"
USER="${3:-admin}"
PASS="${4:-}"
JAR="$(mktemp)"
FAILED=0

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; FAILED=$((FAILED+1)); }

echo "面板: $PANEL"
echo "下游: $DOWN"

echo ""
echo "[1] 面板静态资源"
code=$(curl -sS -o /dev/null -w '%{http_code}' "$PANEL/")
[ "$code" = "200" ] && ok "GET / → $code" || bad "GET / → $code（静态资源没打包进去？）"

code=$(curl -sS -o /dev/null -w '%{http_code}' "$PANEL/some/deep/route")
[ "$code" = "200" ] && ok "SPA 回落 → $code" || bad "SPA 回落 → $code（刷新子路径会 404）"

echo ""
echo "[2] 未登录必须被拦住"
code=$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$PANEL/api/gateway/status" \
  -H 'Content-Type: application/json' -d '{}')
[ "$code" = "401" ] && ok "未登录访问管理 API → 401" || bad "未登录访问管理 API → $code（管理面没设防！）"

echo ""
echo "[3] 登录"
if [ -z "$PASS" ]; then
  bad "没给密码，跳过登录相关检查（用法见脚本头部）"
else
  code=$(curl -sS -o /dev/null -w '%{http_code}' -c "$JAR" -X POST "$PANEL/api/auth/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}")
  [ "$code" = "200" ] && ok "登录 → $code" || bad "登录 → $code"
fi

if [ -s "$JAR" ]; then
  echo ""
  echo "[4] 管理 API"
  for ep in status list-channels list-accounts list-routes paths; do
    body=$(curl -sS -b "$JAR" -X POST "$PANEL/api/gateway/$ep" \
      -H 'Content-Type: application/json' -d '{}' 2>/dev/null || echo '')
    case "$body" in
      *'"code":0'*) ok "$ep" ;;
      *)            bad "$ep → $(echo "$body" | head -c 120)" ;;
    esac
  done

  echo ""
  echo "[5] 下游鉴权状态"
  auth=$(curl -sS -b "$JAR" -X POST "$PANEL/api/gateway/status" \
    -H 'Content-Type: application/json' -d '{}' | tr ',' '\n' | grep '"auth"' || true)
  case "$auth" in
    *api_key*) ok "已配置 API Key（$auth）" ;;
    *open*)    bad "auth=open：任何能访问 $DOWN 的人都能白用号池，公网部署请先在面板加 API Key" ;;
    *)         bad "读不到鉴权状态：$auth" ;;
  esac
fi

echo ""
echo "[6] 下游面"
body=$(curl -sS "$DOWN/v1/models" 2>/dev/null || echo '')
case "$body" in
  *'"object":"list"'*) ok "/v1/models 可访问" ;;
  *401*|*"authentication_error"*) ok "/v1/models 要求鉴权（正常，说明 Key 已生效）" ;;
  *) bad "/v1/models → $(echo "$body" | head -c 120)" ;;
esac

rm -f "$JAR"
echo ""
if [ "$FAILED" -eq 0 ]; then
  printf '\033[32m全部通过\033[0m\n'
else
  printf '\033[31m%s 项未通过\033[0m\n' "$FAILED"
  exit 1
fi
