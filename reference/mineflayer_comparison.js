const mineflayer = require('mineflayer')
const fs = require('fs')

const host = process.env.MC_HOST || '127.0.0.1'
const port = Number(process.env.MC_PORT || 25565)
const count = Number(process.env.BOT_COUNT || 10)
const prefix = process.env.BOT_PREFIX || 'MineJS'
const durationSeconds = Number(process.env.DURATION_SECS || 21600)
const reportPath = process.env.REPORT_PATH || '../reports/mineflayer-comparison.md'
const started = Date.now()
const states = new Map()
if (fs.existsSync(`${reportPath}.invalid.log`)) fs.unlinkSync(`${reportPath}.invalid.log`)

function writeReport () {
  const elapsed = (Date.now() - started) / 1000
  const connected = [...states.values()].filter(state => state.connected).length
  const spawned = [...states.values()].filter(state => state.spawned).length
  const disconnects = [...states.values()].reduce((sum, state) => sum + state.disconnects, 0)
  const errors = [...states.values()].reduce((sum, state) => sum + state.errors, 0)
  const report = `# Mineflayer comparison

- Status: ${elapsed >= durationSeconds ? 'complete' : 'running'}
- Bots: ${count}
- Connected: ${connected}
- Spawned: ${spawned}
- Duration: ${elapsed.toFixed(3)} seconds
- Disconnects: ${disconnects}
- Errors: ${errors}
- Input: forward, even-index sprint, 400ms jump ON / 100ms OFF
- Version: 1.16.1
`
  fs.writeFileSync(`${reportPath}.tmp`, report)
  fs.renameSync(`${reportPath}.tmp`, reportPath)
}

function startInput (bot, index) {
  let yaw = index * (Math.PI * 2 / count)
  let stationaryTicks = 0
  let ticksUntilTurn = 40 + (index % 30)
  let jumpPhase = 0
  let last = bot.entity.position.clone()
  let seed = BigInt(index + 1) * 0x9e3779b97f4a7c15n
  bot.look(yaw, 0, true)
  bot.setControlState('forward', true)
  bot.setControlState('sprint', index % 2 === 0)
  let updating = false
  return setInterval(async () => {
    if (updating) return
    updating = true
    if (!bot.entity) {
      updating = false
      return
    }
    const distance = bot.entity.position.distanceTo(last)
    stationaryTicks = distance < 0.05 ? stationaryTicks + 1 : 0
    last = bot.entity.position.clone()
    ticksUntilTurn--
    const stuck = stationaryTicks >= 30
    if (stuck || ticksUntilTurn <= 0) {
      seed = (seed * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n)
      const degrees = stuck ? 120 + Number(seed % 121n) : 45 + Number(seed % 136n)
      yaw = (yaw + degrees * Math.PI / 180) % (Math.PI * 2)
      await bot.look(yaw, 0, true)
      stationaryTicks = 0
      ticksUntilTurn = 40 + Number((seed >> 16n) % 41n)
    }
    bot.setControlState('jump', jumpPhase < 4)
    jumpPhase = (jumpPhase + 1) % 5
    updating = false
  }, 100)
}

function connectBot (index) {
  const username = `${prefix}${String(index).padStart(2, '0')}`
  const state = { connected: false, spawned: false, disconnects: 0, errors: 0, timer: null, invalidPackets: [] }
  states.set(username, state)
  const bot = mineflayer.createBot({ host, port, username, version: '1.16.1', auth: 'offline' })
  const write = bot._client.write.bind(bot._client)
  bot._client.write = (name, data) => {
    if (['position', 'position_look'].includes(name)) {
      const values = [data.x, data.y, data.z]
      if (values.some(value => !Number.isFinite(value))) {
        state.invalidPackets.push({ name, data: { ...data } })
        fs.appendFileSync(`${reportPath}.invalid.log`, `${username} ${name} ${JSON.stringify(data)}\n`)
      }
    }
    return write(name, data)
  }
  bot.once('spawn', () => {
    state.connected = true
    state.spawned = true
    state.timer = startInput(bot, index)
  })
  bot.on('death', () => bot.respawn())
  bot.on('end', reason => {
    if ((Date.now() - started) / 1000 < durationSeconds) state.disconnects++
    state.connected = false
    state.spawned = false
    if (state.timer) clearInterval(state.timer)
  })
  bot.on('error', () => { state.errors++ })
  state.bot = bot
}

for (let index = 0; index < count; index++) {
  setTimeout(() => connectBot(index), index * 750)
}

writeReport()
const reporter = setInterval(writeReport, 10000)
setTimeout(() => {
  clearInterval(reporter)
  for (const state of states.values()) state.bot?.quit('comparison complete')
  writeReport()
  setTimeout(() => process.exit(0), 1000)
}, durationSeconds * 1000)
