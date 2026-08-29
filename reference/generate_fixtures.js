const fs = require('fs')
const { Physics, PlayerState } = require('prismarine-physics')
const { Vec3 } = require('vec3')
const mcData = require('minecraft-data')('1.16.1')
const Block = require('prismarine-block')('1.16.1')

function makeWorld (extra = () => null) {
  return { getBlock: (pos) => {
    const name = extra(pos.floored()) || (pos.y < 1 ? 'stone' : 'air')
    const block = Block.fromStateId(mcData.blocksByName[name].defaultState, 0)
    block.position = pos.floored()
    return block
  } }
}

function run (name, controlsForTick, ticks, startY = 1, extra = () => null) {
  const world = makeWorld(extra)
  const controls = { forward: false, back: false, left: false, right: false, jump: false, sprint: false, sneak: false }
  const player = {
    entity: {
      position: new Vec3(0.5, startY, 0.5), velocity: new Vec3(0, 0, 0),
      onGround: startY === 1, isInWater: false, isInLava: false, isInWeb: false,
      isCollidedHorizontally: false, isCollidedVertically: false,
      elytraFlying: false, yaw: Math.PI, pitch: 0, effects: {}
    },
    jumpTicks: 0, jumpQueued: false, fireworkRocketDuration: 0,
    version: '1.16.1', inventory: { slots: [] }
  }
  const physics = Physics(mcData, world)
  const state = new PlayerState(player, controls)
  const trajectory = []
  let fallDistance = 0
  for (let tick = 0; tick < ticks; tick++) {
    Object.assign(state.control, controlsForTick(tick))
    const previousY = player.entity.position.y
    physics.simulatePlayer(state, world).apply(player)
    if (player.entity.onGround) fallDistance = 0
    else if (player.entity.position.y < previousY) fallDistance += previousY - player.entity.position.y
    trajectory.push({
      tick: tick + 1,
      position: [player.entity.position.x, player.entity.position.y, player.entity.position.z],
      velocity: [player.entity.velocity.x, player.entity.velocity.y, player.entity.velocity.z],
      on_ground: player.entity.onGround,
      collided_horizontal: player.entity.isCollidedHorizontally,
      collided_vertical: player.entity.isCollidedVertically,
      fall_distance: fallDistance,
      sprinting: Boolean(state.control.sprint)
    })
  }
  return { name, trajectory }
}

const fixtures = [
  run('idle', () => ({}), 40),
  run('walk', () => ({ forward: true }), 40),
  run('jump', tick => ({ jump: tick === 0 }), 40),
  run('sprint', () => ({ forward: true, sprint: true }), 40),
  run('fall', () => ({}), 40, 8),
  run('wall', () => ({ forward: true }), 40, 1, pos => (pos.z === 4 && pos.y === 1) ? 'stone' : null),
  run('slab', () => ({ forward: true }), 30, 1, pos => (pos.z === 2 && pos.y === 1) ? 'oak_slab' : null),
  run('stairs', () => ({ forward: true }), 30, 1, pos => (pos.z === 2 && pos.y === 1) ? 'oak_stairs' : null)
]
fs.writeFileSync(process.argv[2], JSON.stringify({ generator: 'prismarine-physics@1.11.1', minecraft: '1.16.1', fixtures }, null, 2) + '\n')
