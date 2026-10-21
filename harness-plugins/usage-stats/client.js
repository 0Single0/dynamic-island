window.__ModuleLoader__.load({
  id: '@local/usage-stats',
  factory(require) {
    const React = require('react')
    const h = React.createElement

    /** Dictionary namespace owned by this plugin. */
    const NS = 'usageStats'
    /** Shared by the sidebar entry and the main panel it opens. */
    const PANEL_ID = 'usage-stats'
    /** Host channel registered by this bundle's Host half. */
    const CHANNEL = '/usage-stats'

    const BASE = {
      color: 'var(--dsw-alias-label-primary)',
      fontFamily: 'inherit',
    }
    const MUTED = 'var(--dsw-alias-label-secondary)'

    /** Trend series, in legend order. The total line sits on top. */
    const TREND_SERIES = [
      { key: 'total', label: 'trend.total', color: '#10B981' },
      { key: 'cacheRead', label: 'trend.cacheRead', color: '#3B82F6' },
      { key: 'uncachedInput', label: 'trend.uncachedInput', color: '#6366F1' },
      { key: 'output', label: 'trend.output', color: '#EC4899' },
    ]

    /** Donut slice palette (data-viz artwork, deliberately literal). */
    const SLICES = ['#4D6BFE', '#22C55E', '#F59E0B', '#A855F7', '#EC4899', '#06B6D4', '#64748B']

    /** Chart plot box inside the SVG viewBox. */
    const CHART = { width: 720, height: 264, top: 14, right: 16, bottom: 26, left: 54 }

    const en = {
      panel: 'Usage',
      title: 'Usage statistics',
      subtitle: 'Token consumption, cache efficiency, and per-model distribution from the durable session log.',
      refresh: 'Refresh',
      sessions: 'Sessions',
      calls: 'Model calls',
      generated: 'Updated {time}',
      loading: 'Reading session logs…',
      empty: 'No usage recorded in this range.',
      emptyHint: 'Run a turn and refresh; figures come from provider-reported usage in the session log.',
      error: 'Could not read usage',
      retry: 'Retry',
      'range.label': 'Select time range',
      'range.1': 'Last 24 hours',
      'range.7': 'Last 7 days',
      'range.30': 'Last 30 days',
      'range.90': 'Last 90 days',
      'range.0': 'All time',
      'metric.total': 'Total tokens',
      'metric.input': 'Input',
      'metric.output': 'Output',
      'metric.cacheRead': 'Cache hits',
      'metric.cacheWrite': 'Cache writes',
      'metric.hitRate': 'Cache hit rate',
      'metric.hitRateNote': 'cache reads / prompt tokens',
      'chart.channel': 'Usage by channel',
      'chart.channelNote': 'Share of total tokens per provider',
      'chart.trend': 'Usage trend',
      'chart.trendNote': 'Hover the plot to inspect one bucket',
      'chart.models': 'Usage by model',
      'chart.modelsNote': 'Sorted by total tokens',
      'trend.total': 'Total',
      'trend.cacheRead': 'Cache hits',
      'trend.uncachedInput': 'Input',
      'trend.output': 'Output',
      'trend.unit': 'tokens',
      'trend.dateHeading': '{month}/{day}',
      'series.uncached': 'Uncached input',
      'series.cacheRead': 'Cache hit',
      'series.cacheWrite': 'Cache write',
      'series.output': 'Output',
      'table.model': 'Model',
      'table.provider': 'Channel',
      'table.total': 'Total',
      'table.share': 'Share',
      'table.calls': 'Calls',
      'table.hitRate': 'Hit rate',
      'unknown': 'unknown',
      'noData': 'No data in this range',
      'day.calls': '{calls} calls',
      'dot.calls': 'calls',
    }

    const zh = {
      panel: '用量',
      title: '用量统计',
      subtitle: '来自持久化会话日志的 Token 消耗、缓存效率与模型分布。',
      refresh: '刷新',
      sessions: '会话',
      calls: '模型调用',
      generated: '更新于 {time}',
      loading: '正在读取会话日志…',
      empty: '该时间范围内没有用量记录。',
      emptyHint: '运行一轮对话后刷新；数据来自会话日志中服务商上报的用量。',
      error: '读取用量失败',
      retry: '重试',
      'range.label': '选择时间范围',
      'range.1': '近 24 小时',
      'range.7': '近 7 天',
      'range.30': '近 30 天',
      'range.90': '近 90 天',
      'range.0': '全部',
      'metric.total': '总 Token',
      'metric.input': '输入',
      'metric.output': '输出',
      'metric.cacheRead': '缓存命中',
      'metric.cacheWrite': '缓存写入',
      'metric.hitRate': '缓存命中率',
      'metric.hitRateNote': '命中读 / 提示词 Token',
      'chart.channel': '渠道用量',
      'chart.channelNote': '各渠道占总 Token 的比例',
      'chart.trend': '使用趋势',
      'chart.trendNote': '将指针移入图表可查看该时段明细',
      'chart.models': '按模型分布',
      'chart.modelsNote': '按总 Token 排序',
      'trend.total': '总量',
      'trend.cacheRead': '缓存命中',
      'trend.uncachedInput': '输入',
      'trend.output': '输出',
      'trend.unit': 'tokens',
      'trend.dateHeading': '{month}月{day}日',
      'series.uncached': '未缓存输入',
      'series.cacheRead': '缓存命中',
      'series.cacheWrite': '缓存写入',
      'series.output': '输出',
      'table.model': '模型',
      'table.provider': '渠道',
      'table.total': '总量',
      'table.share': '占比',
      'table.calls': '调用次数',
      'table.hitRate': '命中率',
      'unknown': '未知',
      'noData': '该范围内无数据',
      'day.calls': '{calls} 次调用',
      'dot.calls': '次调用',
    }

    /**
     * Compact a number the way the host's counters do.
     * @param {number} value - raw count.
     * @returns {string} short form.
     */
    function compact(value) {
      if (!Number.isFinite(value) || value <= 0) return '0'
      if (value < 1000) return String(Math.round(value))
      if (value < 1e6) return `${(value / 1e3).toFixed(value < 1e4 ? 1 : 0)}K`
      if (value < 1e9) return `${(value / 1e6).toFixed(value < 1e7 ? 2 : 1)}M`
      return `${(value / 1e9).toFixed(2)}B`
    }

    /**
     * Group-separated exact count for titles and tables.
     * @param {number} value - raw count.
     * @returns {string} exact form.
     */
    function exact(value) {
      return Number.isFinite(value) ? Math.round(value).toLocaleString() : '0'
    }

    /**
     * Percent form of a 0..1 ratio.
     * @param {number|null} ratio - ratio, or null when no prompt tokens exist.
     * @param {number} digits - fraction digits.
     * @returns {string} percent text.
     */
    function percent(ratio, digits) {
      if (ratio === null || !Number.isFinite(ratio)) return '—'
      return `${(ratio * 100).toFixed(digits === undefined ? 1 : digits)}%`
    }

    /**
     * Local wall-clock time for the "updated" line.
     * @param {number} time - epoch milliseconds.
     * @returns {string} HH:MM:SS.
     */
    function clock(time) {
      const date = new Date(time)
      return Number.isNaN(date.getTime()) ? '—' : date.toLocaleTimeString()
    }

    /**
     * Pick a round axis maximum and step so ticks land on nice numbers.
     * @param {number} max - data maximum.
     * @returns {{ top: number, ticks: number[] }} axis top and tick values.
     */
    function niceAxis(max) {
      if (!Number.isFinite(max) || max <= 0) return { top: 1, ticks: [0, 1] }
      const rough = max / 4
      const magnitude = Math.pow(10, Math.floor(Math.log10(rough)))
      const normalized = rough / magnitude
      const step = (normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10) * magnitude
      const top = Math.ceil(max / step) * step
      const ticks = []
      for (let value = 0; value <= top + step / 2; value += step) ticks.push(value)
      return { top, ticks }
    }

    /** Sign helper used by the monotone tangent rule. */
    function sign(value) {
      return value < 0 ? -1 : 1
    }

    /**
     * Tangent slope at one point of a monotone cubic.
     *
     * This is d3's `curveMonotoneX` rule. Its defining property: when the two
     * neighbouring slopes disagree in sign the point is a local extremum, so the
     * sum is zero and the tangent is zero — the curve flattens at the peak
     * instead of sailing past it. Otherwise the tangent is clamped against both
     * neighbouring secants, which is what keeps the drawn curve inside its data.
     *
     * A plain Catmull-Rom spline (what this replaced) has no such clamp, so a
     * spike next to a run of zeros overshoots below zero.
     *
     * @param {number} h0 - pixel width of the segment entering this point.
     * @param {number} h1 - pixel width of the segment leaving this point.
     * @param {number} s0 - slope entering this point.
     * @param {number} s1 - slope leaving this point.
     * @returns {number} the tangent slope at this point.
     */
    function monotoneTangent(h0, h1, s0, s1) {
      if (h0 <= 0 || h1 <= 0) return s1
      const mean = (s0 * h1 + s1 * h0) / (h0 + h1)
      const limit = Math.min(Math.abs(s0), Math.abs(s1), 0.5 * Math.abs(mean))
      return (sign(s0) + sign(s1)) * limit
    }

    /**
     * Build a monotone cubic path — the shape a chart library draws for
     * `type="monotone"`. Cubic Hermite segments are emitted as Béziers with
     * control points one third of the way along each segment.
     * @param {Array<{ x: number, y: number }>} points - plot-space points.
     * @returns {string} SVG path data.
     */
    function smoothPath(points) {
      const count = points.length
      if (count === 0) return ''
      if (count === 1) return `M ${points[0].x} ${points[0].y}`

      // One tangent per point: one-sided at the ends, the monotone rule inside.
      const tangent = new Array(count)
      for (let index = 0; index < count; index += 1) {
        const previous = points[index - 1]
        const current = points[index]
        const next = points[index + 1]
        if (previous === undefined) {
          tangent[index] = next.x === current.x ? 0 : (next.y - current.y) / (next.x - current.x)
          continue
        }
        if (next === undefined) {
          tangent[index] = current.x === previous.x ? 0 : (current.y - previous.y) / (current.x - previous.x)
          continue
        }
        const h0 = current.x - previous.x
        const h1 = next.x - current.x
        tangent[index] = monotoneTangent(
          h0,
          h1,
          h0 === 0 ? 0 : (current.y - previous.y) / h0,
          h1 === 0 ? 0 : (next.y - current.y) / h1,
        )
      }

      let path = `M ${points[0].x} ${points[0].y}`
      for (let index = 0; index < count - 1; index += 1) {
        const from = points[index]
        const to = points[index + 1]
        const third = (to.x - from.x) / 3
        const c1x = from.x + third
        const c1y = from.y + tangent[index] * third
        const c2x = to.x - third
        const c2y = to.y - tangent[index + 1] * third
        path += ` C ${c1x.toFixed(2)} ${c1y.toFixed(2)}, ${c2x.toFixed(2)} ${c2y.toFixed(2)}, ${to.x.toFixed(2)} ${to.y.toFixed(2)}`
      }
      return path
    }

    /** One rounded surface card. */
    function Card(props) {
      return h('section', {
        style: {
          background: 'var(--dsw-alias-bg-layer-1)',
          border: '1px solid var(--dsw-alias-border-l1)',
          borderRadius: 12,
          padding: 16,
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
          minWidth: 0,
        },
      }, props.children)
    }

    /** Section heading with an optional note. */
    function SectionHead(props) {
      return h('header', { style: { display: 'flex', flexDirection: 'column', gap: 2 } },
        h('h2', { style: { margin: 0, fontSize: 13, fontWeight: 600, color: BASE.color } }, props.title),
        props.note === undefined ? null : h('span', { style: { fontSize: 11, color: MUTED } }, props.note),
      )
    }

    /** One KPI tile. */
    function Metric(props) {
      return h('div', {
        title: props.hint,
        style: {
          background: 'var(--dsw-alias-bg-layer-1)',
          border: '1px solid var(--dsw-alias-border-l1)',
          borderRadius: 10,
          padding: '12px 14px',
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          minWidth: 0,
        },
      },
        h('span', { style: { fontSize: 11, color: MUTED } }, props.label),
        h('span', { style: { fontSize: 21, fontWeight: 600, letterSpacing: '-0.01em', color: BASE.color } }, props.value),
        props.note === undefined ? null : h('span', { style: { fontSize: 10, color: MUTED } }, props.note),
      )
    }

    /** Donut chart with a legend; segments are stroke dashes on one circle. */
    function Donut(props) {
      const { entries, t } = props
      const total = entries.reduce((sum, entry) => sum + entry.value, 0)
      const radius = 52
      const circumference = 2 * Math.PI * radius
      let offset = 0
      const arcs = entries.map((entry, index) => {
        const share = total > 0 ? entry.value / total : 0
        const dash = share * circumference
        const arc = h('circle', {
          key: entry.label,
          cx: 64,
          cy: 64,
          r: radius,
          fill: 'none',
          stroke: SLICES[index % SLICES.length],
          strokeWidth: 16,
          strokeDasharray: `${dash} ${circumference - dash}`,
          strokeDashoffset: -offset,
          transform: 'rotate(-90 64 64)',
        })
        offset += dash
        return arc
      })
      return h('div', { style: { display: 'flex', gap: 20, alignItems: 'center', flexWrap: 'wrap' } },
        h('div', { style: { position: 'relative', width: 128, height: 128, flex: '0 0 auto' } },
          h('svg', { viewBox: '0 0 128 128', width: 128, height: 128, role: 'img', 'aria-label': props.ariaLabel },
            h('circle', { cx: 64, cy: 64, r: radius, fill: 'none', stroke: 'var(--dsw-alias-bg-layer-2)', strokeWidth: 16 }),
            arcs,
          ),
          h('div', {
            style: {
              position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center', gap: 1, pointerEvents: 'none',
            },
          },
            h('span', { style: { fontSize: 15, fontWeight: 600, color: BASE.color } }, compact(total)),
            h('span', { style: { fontSize: 10, color: MUTED } }, t('metric.total')),
          ),
        ),
        h('ul', {
          style: { listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 7, flex: '1 1 160px', minWidth: 0 },
        }, entries.map((entry, index) => h('li', {
          key: entry.label,
          style: { display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, minWidth: 0 },
        },
          h('span', {
            style: {
              width: 9, height: 9, borderRadius: 3, flex: '0 0 auto',
              background: SLICES[index % SLICES.length],
            },
          }),
          h('span', {
            title: entry.label,
            style: { color: BASE.color, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: '1 1 auto' },
          }, entry.label),
          h('span', { style: { color: MUTED, fontVariantNumeric: 'tabular-nums' } },
            `${percent(total > 0 ? entry.value / total : null, 1)}`),
        ))),
      )
    }

    /**
     * Multi-series smooth area chart with a hover crosshair and tooltip.
     *
     * @param {object} props - `rows` (buckets) and `hourly` granularity.
     * @returns {import('react').ReactElement} the chart element.
     */
    function TrendChart(props) {
      const rows = Array.isArray(props.rows) ? props.rows : []
      const hourly = props.hourly === true
      const t = props.t
      const [hover, setHover] = React.useState(-1)
      const wrapRef = React.useRef(null)

      const plotWidth = CHART.width - CHART.left - CHART.right
      const plotHeight = CHART.height - CHART.top - CHART.bottom
      const stepX = rows.length > 1 ? plotWidth / (rows.length - 1) : 0
      const xs = rows.map((_, index) => CHART.left + (rows.length > 1 ? index * stepX : plotWidth / 2))

      let peak = 0
      for (const row of rows) for (const series of TREND_SERIES) peak = Math.max(peak, Number(row[series.key]) || 0)
      const axis = niceAxis(peak)
      const yOf = (value) => CHART.top + plotHeight - (Math.max(0, Number(value) || 0) / axis.top) * plotHeight

      /** Nearest data column for a pointer event, or -1 outside the plot. */
      const indexAt = (event) => {
        const node = event === null || event === undefined ? undefined : event.currentTarget
        if (node === null || node === undefined || rows.length === 0) return -1
        const rect = node.getBoundingClientRect()
        if (rect.width <= 0) return -1
        const viewX = ((event.clientX - rect.left) / rect.width) * CHART.width
        if (viewX < CHART.left - stepX / 2 || viewX > CHART.width - CHART.right + stepX / 2) return -1
        if (rows.length === 1) return 0
        return Math.max(0, Math.min(rows.length - 1, Math.round((viewX - CHART.left) / stepX)))
      }

      const active = hover >= 0 && hover < rows.length ? rows[hover] : null
      const activeX = hover >= 0 && hover < xs.length ? xs[hover] : 0

      // Tooltip box: flip left of the crosshair near the right edge.
      const tipWidth = 200
      const tipHeight = 32 + TREND_SERIES.length * 20
      const tipX = activeX + 16 + tipWidth > CHART.width - CHART.right ? activeX - 16 - tipWidth : activeX + 16
      const tipY = CHART.top + 6

      /** Axis label for one bucket key. */
      const tickLabel = (key) => {
        if (typeof key !== 'string') return ''
        if (hourly) return key.length >= 13 ? `${key.slice(11, 13)}:00` : key
        return `${key.slice(5, 7)}/${key.slice(8, 10)}`
      }

      /** Tooltip heading: `10月20日 10:00` when hourly, the date otherwise. */
      const heading = (key) => {
        if (typeof key !== 'string') return ''
        const parts = key.split('T')
        const date = parts[0]
        const head = t('trend.dateHeading', { month: String(Number(date.slice(5, 7))), day: String(Number(date.slice(8, 10))) })
        return hourly && parts[1] !== undefined ? `${head} ${parts[1]}:00` : head
      }

      const grid = axis.ticks.map((value) => {
        const y = yOf(value)
        return h('g', { key: `tick-${value}` },
          h('line', {
            x1: CHART.left, x2: CHART.width - CHART.right, y1: y, y2: y,
            stroke: 'var(--dsw-alias-border-l1)', strokeWidth: 1,
            strokeDasharray: value === 0 ? '0' : '3 4',
          }),
          h('text', {
            x: CHART.left - 10, y: y + 3.5, textAnchor: 'end',
            fill: MUTED, fontSize: 10.5,
            style: { fontVariantNumeric: 'tabular-nums' },
          }, compact(value)),
        )
      })

      const labelEvery = Math.max(1, Math.ceil(rows.length / 6))
      const xTicks = []
      rows.forEach((row, index) => {
        if (index % labelEvery !== 0 && index !== rows.length - 1) return
        xTicks.push(h('text', {
          key: `x-${row.key}`,
          x: xs[index], y: CHART.height - CHART.bottom + 17, textAnchor: 'middle',
          fill: MUTED, fontSize: 10.5,
        }, tickLabel(row.key)))
      })

      // Lower series first so the total line stays on top.
      const layers = TREND_SERIES.slice().reverse().map((series) => {
        const points = rows.map((row, index) => ({ x: xs[index], y: yOf(row[series.key]) }))
        const line = smoothPath(points)
        if (line === '') return null
        const baseY = (CHART.top + plotHeight).toFixed(2)
        const area = `${line} L ${points[points.length - 1].x.toFixed(2)} ${baseY} L ${points[0].x.toFixed(2)} ${baseY} Z`
        const gradientId = `usage-grad-${series.key}`
        return h('g', { key: series.key },
          h('linearGradient', { id: gradientId, x1: 0, y1: 0, x2: 0, y2: 1 },
            h('stop', { offset: '0%', stopColor: series.color, stopOpacity: 0.26 }),
            h('stop', { offset: '100%', stopColor: series.color, stopOpacity: 0.02 }),
          ),
          h('path', { d: area, fill: `url(#${gradientId})`, stroke: 'none' }),
          h('path', {
            d: line, fill: 'none', stroke: series.color, strokeWidth: 2,
            strokeLinecap: 'round', strokeLinejoin: 'round',
          }),
        )
      }).filter(Boolean)

      const markers = active === null ? null : h('g', null,
        h('line', {
          x1: activeX, x2: activeX, y1: CHART.top, y2: CHART.top + plotHeight,
          stroke: MUTED, strokeWidth: 1, strokeDasharray: '4 4', opacity: 0.7,
        }),
        TREND_SERIES.map((series) => h('circle', {
          key: series.key,
          cx: activeX, cy: yOf(active[series.key]), r: 3.5,
          fill: series.color,
          stroke: 'var(--dsw-alias-bg-layer-1)',
          strokeWidth: 1.5,
        })),
      )

      const tooltip = active === null ? null : h('g', { pointerEvents: 'none' },
        h('rect', {
          x: tipX, y: tipY, width: tipWidth, height: tipHeight, rx: 9,
          fill: 'var(--dsw-alias-bg-overlay)',
          stroke: 'var(--dsw-alias-border-l2)',
          strokeWidth: 1,
          style: { filter: 'drop-shadow(0 6px 16px rgba(0,0,0,0.12))' },
        }),
        h('text', {
          x: tipX + 12, y: tipY + 20,
          fill: BASE.color, fontSize: 11.5, fontWeight: 600,
        }, heading(active.key)),
        TREND_SERIES.map((series, index) => {
          const rowY = tipY + 40 + index * 20
          return h('g', { key: `tip-${series.key}` },
            h('circle', { cx: tipX + 17, cy: rowY - 3.5, r: 4, fill: series.color }),
            h('text', { x: tipX + 29, y: rowY + 1, fill: MUTED, fontSize: 11 }, t(series.label)),
            h('text', {
              x: tipX + tipWidth - 12, y: rowY + 1, textAnchor: 'end',
              fill: BASE.color, fontSize: 11,
              style: { fontVariantNumeric: 'tabular-nums' },
            }, `${compact(active[series.key])} ${t('trend.unit')}`),
          )
        }),
      )

      return h('div', {
        ref: wrapRef,
        style: { display: 'flex', flexDirection: 'column', gap: 4 },
      },
        h('svg', {
          viewBox: `0 0 ${CHART.width} ${CHART.height}`,
          role: 'img',
          'aria-label': t('chart.trend'),
          style: { display: 'block', width: '100%', height: 'auto', overflow: 'visible' },
          onPointerMove: (event) => setHover(indexAt(event)),
          onPointerLeave: () => setHover(-1),
        },
          grid,
          xTicks,
          layers,
          markers,
          tooltip,
          h('rect', {
            x: CHART.left, y: CHART.top,
            width: plotWidth, height: plotHeight,
            fill: 'transparent',
          }),
        ),
        h('div', {
          style: {
            display: 'flex', justifyContent: 'center', gap: 18, flexWrap: 'wrap',
            fontSize: 11, color: MUTED, paddingTop: 4,
          },
        }, TREND_SERIES.map((series) => h('span', {
          key: series.key,
          style: { display: 'inline-flex', alignItems: 'center', gap: 7 },
        },
          h('span', {
            style: {
              width: 15, height: 3, borderRadius: 2,
              background: series.color, display: 'inline-block',
            },
          }),
          t(series.label),
        ))),
      )
    }

    /** Per-model rows with a proportional bar and the four buckets. */
    function ModelTable(props) {
      const { models, t } = props
      const max = models.reduce((peak, row) => Math.max(peak, row.total), 0)
      return h('div', { style: { display: 'flex', flexDirection: 'column', gap: 2 } },
        h('div', {
          style: {
            display: 'grid',
            gridTemplateColumns: 'minmax(140px, 2fr) minmax(64px, 1fr) 76px 84px 64px',
            gap: 10, padding: '0 8px 6px', fontSize: 10, color: MUTED,
          },
        },
          h('span', null, t('table.model')),
          h('span', null, t('table.provider')),
          h('span', { style: { textAlign: 'right' } }, t('table.total')),
          h('span', null, t('table.share')),
          h('span', { style: { textAlign: 'right' } }, t('table.hitRate')),
        ),
        models.map((row) => {
          const share = max > 0 ? row.total / max : 0
          return h('div', {
            key: `${row.provider}\u0000${row.model}`,
            title: `${row.model}\n${t('table.provider')}: ${row.provider}\n${t('series.uncached')}: ${exact(row.uncachedInput)}\n${t('series.cacheRead')}: ${exact(row.cacheRead)}\n${t('series.cacheWrite')}: ${exact(row.cacheWrite)}\n${t('series.output')}: ${exact(row.output)}\n${t('table.calls')}: ${exact(row.calls)}`,
            style: {
              display: 'grid',
              gridTemplateColumns: 'minmax(140px, 2fr) minmax(64px, 1fr) 76px 84px 64px',
              gap: 10, alignItems: 'center', padding: '7px 8px', borderRadius: 8, fontSize: 12,
            },
          },
            h('span', {
              style: { color: BASE.color, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: 500 },
            }, row.model),
            h('span', {
              style: { color: MUTED, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
            }, row.provider),
            h('span', {
              style: { textAlign: 'right', color: BASE.color, fontVariantNumeric: 'tabular-nums' },
            }, compact(row.total)),
            h('span', {
              style: {
                height: 6, borderRadius: 3, background: 'var(--dsw-alias-bg-layer-2)', overflow: 'hidden',
              },
            }, h('span', {
              style: {
                display: 'block', height: '100%', width: `${Math.max(share * 100, row.total > 0 ? 2 : 0)}%`,
                borderRadius: 3, background: 'var(--dsw-alias-brand-primary)',
              },
            })),
            h('span', {
              style: { textAlign: 'right', color: MUTED, fontVariantNumeric: 'tabular-nums' },
            }, percent(row.hitRate, 1)),
          )
        }),
      )
    }

    /** Range segmented control. */
    function RangePicker(props) {
      const { value, onChange, t } = props
      const options = [1, 7, 30, 90, 0]
      return h('div', {
        role: 'group',
        'aria-label': t('range.label'),
        style: {
          display: 'inline-flex', padding: 2, gap: 2, borderRadius: 9,
          background: 'var(--dsw-alias-bg-layer-2)', border: '1px solid var(--dsw-alias-border-l1)',
        },
      }, options.map((option) => {
        const active = value === option
        return h('button', {
          key: String(option),
          type: 'button',
          'aria-pressed': active,
          onClick: () => onChange(option),
          style: {
            border: 'none', cursor: 'pointer', borderRadius: 7, padding: '5px 10px',
            fontSize: 12, fontFamily: 'inherit',
            background: active ? 'var(--dsw-alias-bg-layer-1)' : 'transparent',
            color: active ? BASE.color : MUTED,
            boxShadow: active ? '0 1px 2px rgba(0,0,0,0.08)' : 'none',
          },
        }, t(`range.${option}`))
      }))
    }

    /** The Usage panel body. */
    function UsagePage(props) {
      const t = props.t
      const [days, setDays] = React.useState(30)
      const [state, setState] = React.useState({ status: 'loading', data: null, error: null })
      const [nonce, setNonce] = React.useState(0)
      const requestId = React.useRef(0)

      React.useEffect(() => {
        const id = requestId.current + 1
        requestId.current = id
        setState((previous) => ({ status: 'loading', data: previous.data, error: null }))
        props
          .load({ days, tzOffsetMinutes: new Date().getTimezoneOffset() })
          .then((value) => {
            if (requestId.current !== id) return
            setState({ status: 'ready', data: value, error: null })
          })
          .catch((error) => {
            if (requestId.current !== id) return
            setState({ status: 'error', data: null, error: error instanceof Error ? error.message : String(error) })
          })
      }, [days, nonce])

      const data = state.data
      const loading = state.status === 'loading'
      const refreshed = React.useCallback(() => setNonce((value) => value + 1), [])
      const hourly = data !== null && data.range !== undefined && data.range.bucket === 'hour'

      const channels = React.useMemo(() => {
        if (data === null) return []
        return data.providers.map((row) => ({
          label: row.provider === 'unknown' ? t('unknown') : row.provider,
          value: row.total,
        }))
      }, [data, t])

      return h('div', {
        style: {
          height: '100%', width: '100%', overflowY: 'auto', overflowX: 'hidden',
          color: BASE.color,
        },
      },
        h('div', {
          style: { maxWidth: 1040, margin: '0 auto', padding: '26px 24px 40px', display: 'flex', flexDirection: 'column', gap: 16 },
        },
          h('header', { style: { display: 'flex', alignItems: 'flex-start', gap: 12, flexWrap: 'wrap' } },
            h('div', { style: { flex: '1 1 260px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 } },
              h('h1', { style: { margin: 0, fontSize: 19, fontWeight: 600, letterSpacing: '-0.01em' } }, t('title')),
              h('p', { style: { margin: 0, fontSize: 12, color: MUTED } }, t('subtitle')),
            ),
            h('div', { style: { display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' } },
              h(RangePicker, { value: days, onChange: setDays, t }),
              h('button', {
                type: 'button',
                onClick: refreshed,
                title: t('refresh'),
                'aria-label': t('refresh'),
                style: {
                  display: 'inline-flex', alignItems: 'center', gap: 6, cursor: 'pointer',
                  borderRadius: 8, padding: '6px 11px', fontSize: 12, fontFamily: 'inherit',
                  color: BASE.color, background: 'var(--dsw-alias-bg-layer-1)',
                  border: '1px solid var(--dsw-alias-border-l1)',
                  opacity: loading ? 0.6 : 1,
                },
              },
                h('svg', { viewBox: '0 0 16 16', width: 13, height: 13, 'aria-hidden': true },
                  h('path', {
                    d: 'M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2v3h-3',
                    fill: 'none', stroke: 'currentColor', strokeWidth: 1.4,
                    strokeLinecap: 'round', strokeLinejoin: 'round',
                  }),
                ),
                t('refresh'),
              ),
            ),
          ),

          h('div', { style: { display: 'flex', gap: 14, flexWrap: 'wrap', fontSize: 11, color: MUTED, minHeight: 15 } },
            data === null ? null : h('span', null, `${t('sessions')} ${exact(data.sessions)}`),
            data === null ? null : h('span', null, `${t('calls')} ${exact(data.totals.calls)}`),
            data === null ? null : h('span', null, t('generated', { time: clock(data.generatedAt) })),
            loading ? h('span', null, t('loading')) : null,
          ),

          state.status === 'error'
            ? h(Card, null,
              h('div', { style: { display: 'flex', flexDirection: 'column', gap: 8 } },
                h('strong', { style: { color: 'var(--dsw-alias-state-error-primary)', fontSize: 13 } }, t('error')),
                h('span', { style: { fontSize: 12, color: MUTED, whiteSpace: 'pre-wrap', wordBreak: 'break-word' } }, state.error),
                h('button', {
                  type: 'button', onClick: refreshed,
                  style: {
                    alignSelf: 'flex-start', cursor: 'pointer', borderRadius: 8, padding: '6px 12px',
                    fontSize: 12, fontFamily: 'inherit', color: BASE.color,
                    background: 'var(--dsw-alias-bg-layer-2)',
                    border: '1px solid var(--dsw-alias-border-l1)',
                  },
                }, t('retry')),
              ),
            )
            : null,

          h('div', {
            style: {
              display: 'grid', gap: 10,
              gridTemplateColumns: 'repeat(auto-fit, minmax(132px, 1fr))',
              opacity: data === null ? 0.45 : 1, transition: 'opacity 120ms ease',
            },
          },
            h(Metric, {
              label: t('metric.total'), value: compact(data === null ? 0 : data.totals.total),
              hint: data === null ? '' : exact(data.totals.total),
            }),
            h(Metric, {
              label: t('metric.input'), value: compact(data === null ? 0 : data.totals.prompt),
              hint: data === null ? '' : exact(data.totals.prompt),
            }),
            h(Metric, {
              label: t('metric.output'), value: compact(data === null ? 0 : data.totals.output),
              hint: data === null ? '' : exact(data.totals.output),
            }),
            h(Metric, {
              label: t('metric.cacheRead'), value: compact(data === null ? 0 : data.totals.cacheRead),
              hint: data === null ? '' : exact(data.totals.cacheRead),
            }),
            h(Metric, {
              label: t('metric.cacheWrite'), value: compact(data === null ? 0 : data.totals.cacheWrite),
              hint: data === null ? '' : exact(data.totals.cacheWrite),
            }),
            h(Metric, {
              label: t('metric.hitRate'), value: percent(data === null ? null : data.totals.hitRate, 1),
              note: t('metric.hitRateNote'),
            }),
          ),

          // Trend spans the full width, as the wide time series it is.
          h(Card, null,
            h(SectionHead, { title: t('chart.trend'), note: t('chart.trendNote') }),
            data === null || data.series.length === 0
              ? h('div', { style: { fontSize: 12, color: MUTED, padding: '18px 0' } }, t('noData'))
              : h(TrendChart, { rows: data.series, hourly, t }),
          ),

          h('div', {
            style: { display: 'grid', gap: 12, gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))' },
          },
            h(Card, null,
              h(SectionHead, { title: t('chart.channel'), note: t('chart.channelNote') }),
              channels.length === 0
                ? h('div', { style: { fontSize: 12, color: MUTED, padding: '18px 0' } }, t('noData'))
                : h(Donut, { entries: channels, t, ariaLabel: t('chart.channel') }),
            ),
            h(Card, null,
              h(SectionHead, { title: t('chart.models'), note: t('chart.modelsNote') }),
              data === null || data.models.length === 0
                ? h('div', {
                  style: { display: 'flex', flexDirection: 'column', gap: 4, padding: '16px 0' },
                },
                  h('span', { style: { fontSize: 12, color: BASE.color } }, t('empty')),
                  h('span', { style: { fontSize: 11, color: MUTED } }, t('emptyHint')),
                )
                : h(ModelTable, { models: data.models, t }),
            ),
          ),
        ),
      )
    }

    /** Sidebar glyph: three ascending bars. */
    function UsageIcon(props) {
      const size = props.size === undefined ? 18 : props.size
      return h('svg', {
        viewBox: '0 0 16 16', width: size, height: size, 'aria-hidden': true,
        style: { display: 'block', color: 'currentColor' },
      },
        h('path', {
          d: 'M2.75 13.25V8M8 13.25V3M13.25 13.25V6',
          fill: 'none', stroke: 'currentColor', strokeWidth: 1.6, strokeLinecap: 'round',
        }),
      )
    }

    return {
      inject: ['slots', 'locale', 'connection'],
      apply(ctx) {
        ctx.effect(() => ctx.locale.register(NS, { en, zh }), 'usage-stats: dictionaries')
        const t = ctx.locale.bind(NS)
        const load = async (options) => {
          const result = await ctx.connection.rpc.call(CHANNEL, 'summary', options)
          if (result.ok !== true) {
            const error = result.error
            throw new Error(error === undefined ? 'usage request failed' : `${error.message}`)
          }
          return result.value
        }
        const face = { load }
        ctx.slots.inject('main', () => ctx.slots.register({
          name: 'main',
          key: PANEL_ID,
          locale: NS,
          inject: () => face,
        }, UsagePage))
        ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
          name: 'sidebar.panellist',
          id: PANEL_ID,
          order: 20,
          locale: NS,
          label: () => t('panel'),
        }, UsageIcon))
      },
    }
  },
})
