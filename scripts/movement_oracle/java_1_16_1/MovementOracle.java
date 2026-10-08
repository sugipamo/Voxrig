// Movement oracle for Java 1.16.1. Written against Mojang's published names,
// compiled against a locally remapped copy of the official server, then remapped
// back so it runs inside the unchanged official server JAR (see run.py).
// No game code is copied: the player below calls the official movement code
// and only ports the client-side input handling of LocalPlayer, noted inline.
package voxrig.oracle;

import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import net.minecraft.tags.BlockTags;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.FenceGateBlock;
import net.minecraft.world.level.block.TrapDoorBlock;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Registry;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

public final class MovementOracle {
    /** LocalPlayer's Input fields that movement reads. */
    static final class Keys {
        boolean up, down, left, right, jumping, shiftKeyDown, sprint;
    }

    /** A survival player driven like LocalPlayer, inside a server level. */
    static final class OraclePlayer extends Player {
        Keys next = new Keys();
        boolean up, down, left, right, inputJumping, shiftKeyDown, sprintKey;
        float leftImpulse, forwardImpulse;
        boolean crouching;

        OraclePlayer(Level level) {
            super(level, BlockPos.ZERO, new GameProfile(new UUID(0, 1), "oracle"));
            abilities.invulnerable = true;
        }

        @Override public boolean isSpectator() { return false; }
        @Override public boolean isCreative() { return false; }
        // LocalPlayer overrides.
        @Override public boolean isLocalPlayer() { return true; }
        @Override public boolean isEffectiveAi() { return true; }
        @Override public boolean isShiftKeyDown() { return shiftKeyDown; }
        @Override public boolean isCrouching() { return crouching; }
        @Override public boolean isUnderWater() { return wasUnderwater; }
        boolean isMovingSlowly() { return isCrouching() || isVisuallyCrawling(); }
        // Client-only Entity.isVisuallyCrawling and LivingEntity.goDownInWater
        // (stripped from the server JAR).
        boolean isVisuallyCrawling() { return isVisuallySwimming() && !isInWater(); }
        void goDownInWater() { setDeltaMovement(getDeltaMovement().add(0.0, -0.04F, 0.0)); }
        boolean hasForwardImpulse() { return forwardImpulse > 1.0E-5F; }
        boolean hasEnoughImpulseToStartSprinting() {
            return isUnderWater() ? hasForwardImpulse() : forwardImpulse >= 0.8;
        }

        // KeyboardInput.tick(slow).
        void tickInput(boolean slow) {
            up = next.up; down = next.down; left = next.left; right = next.right;
            forwardImpulse = up == down ? 0.0F : up ? 1.0F : -1.0F;
            leftImpulse = left == right ? 0.0F : left ? 1.0F : -1.0F;
            inputJumping = next.jumping;
            shiftKeyDown = next.shiftKeyDown;
            sprintKey = next.sprint;
            if (slow) {
                leftImpulse = (float) (leftImpulse * 0.3);
                forwardImpulse = (float) (forwardImpulse * 0.3);
            }
        }

        // Port of the movement-relevant part of LocalPlayer.aiStep. Sprinting is
        // requested only by the sprint key (no double-tap), flying is not used.
        @Override public void aiStep() {
            boolean wasShift = shiftKeyDown;
            boolean hadImpulse = hasEnoughImpulseToStartSprinting();
            crouching = !abilities.flying && !isSwimming() && canEnterPose(Pose.CROUCHING)
                && (isShiftKeyDown() || !isSleeping() && !canEnterPose(Pose.STANDING));
            tickInput(isMovingSlowly());
            if (isUsingItem() && !isPassenger()) {
                leftImpulse *= 0.2F;
                forwardImpulse *= 0.2F;
            }
            if (!noPhysics) {
                double w = getBbWidth() * 0.35;
                checkInBlock(getX() - w, getY() + 0.5, getZ() + w);
                checkInBlock(getX() - w, getY() + 0.5, getZ() - w);
                checkInBlock(getX() + w, getY() + 0.5, getZ() - w);
                checkInBlock(getX() + w, getY() + 0.5, getZ() + w);
            }
            boolean food = getFoodData().getFoodLevel() > 6.0F || abilities.mayfly;
            if ((onGround || isUnderWater()) && !wasShift && !hadImpulse && hasEnoughImpulseToStartSprinting()
                && !isSprinting() && food && !isUsingItem() && !hasEffect(MobEffects.BLINDNESS) && sprintKey) {
                setSprinting(true);
            }
            if (!isSprinting() && (!isInWater() || isUnderWater()) && hasEnoughImpulseToStartSprinting() && food
                && !isUsingItem() && !hasEffect(MobEffects.BLINDNESS) && sprintKey) {
                setSprinting(true);
            }
            if (isSprinting()) {
                boolean noImpulse = !hasForwardImpulse() || !food;
                boolean stop = noImpulse || horizontalCollision || isInWater() && !isUnderWater();
                if (isSwimming()) {
                    if (!onGround && !shiftKeyDown && noImpulse || !isInWater()) setSprinting(false);
                } else if (stop) {
                    setSprinting(false);
                }
            }
            if (isInWater() && shiftKeyDown && isAffectedByFluids()) goDownInWater();
            super.aiStep();
        }

        // LocalPlayer.serverAiStep: the client copies its input into the entity.
        @Override public void serverAiStep() {
            super.serverAiStep();
            xxa = leftImpulse;
            zza = forwardImpulse;
            jumping = inputJumping;
        }

        // LocalPlayer.checkInBlock and blocked.
        @Override protected void checkInBlock(double x, double y, double z) {
            BlockPos pos = new BlockPos(x, y, z);
            if (!blocked(pos)) return;
            double dx = x - pos.getX(), dz = z - pos.getZ();
            Direction best = null;
            double nearest = 9999.0;
            if (!blocked(pos.west()) && dx < nearest) { nearest = dx; best = Direction.WEST; }
            if (!blocked(pos.east()) && 1.0 - dx < nearest) { nearest = 1.0 - dx; best = Direction.EAST; }
            if (!blocked(pos.north()) && dz < nearest) { nearest = dz; best = Direction.NORTH; }
            if (!blocked(pos.south()) && 1.0 - dz < nearest) { best = Direction.SOUTH; }
            if (best == null) return;
            Vec3 v = getDeltaMovement();
            switch (best) {
                case WEST: setDeltaMovement(-0.1, v.y, v.z); break;
                case EAST: setDeltaMovement(0.1, v.y, v.z); break;
                case NORTH: setDeltaMovement(v.x, v.y, -0.1); break;
                default: setDeltaMovement(v.x, v.y, 0.1);
            }
        }
        boolean blocked(BlockPos pos) {
            AABB box = getBoundingBox();
            BlockPos.MutableBlockPos cursor = pos.mutable();
            for (int y = net.minecraft.util.Mth.floor(box.minY); y < net.minecraft.util.Mth.ceil(box.maxY); y++) {
                cursor.setY(y);
                if (!freeAt(cursor)) return true;
            }
            return false;
        }
    }

    static MinecraftServer findServer() throws Exception {
        for (Thread thread : Thread.getAllStackTraces().keySet()) {
            if (!thread.getName().equals("Server thread")) continue;
            Field holder = Thread.class.getDeclaredField("holder");
            holder.setAccessible(true);
            Object h = holder.get(thread);
            Field task = h.getClass().getDeclaredField("task");
            task.setAccessible(true);
            Object runnable = task.get(h);
            for (Field f : runnable.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                if (f.get(runnable) instanceof AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) return s;
            }
        }
        return null;
    }

    static BlockState parseState(String text) {
        int open = text.indexOf('[');
        String name = open < 0 ? text : text.substring(0, open);
        Block block = Registry.BLOCK.getOptional(new ResourceLocation(name)).orElseThrow(() -> new IllegalArgumentException(name));
        BlockState state = block.defaultBlockState();
        if (open >= 0) {
            for (String pair : text.substring(open + 1, text.length() - 1).split(",")) {
                String[] kv = pair.split("=");
                state = with(state, block.getStateDefinition().getProperty(kv[0]), kv[1]);
            }
        }
        return state;
    }
    static <T extends Comparable<T>> BlockState with(BlockState state, Property<T> property, String value) {
        if (property == null) throw new IllegalArgumentException("property " + value);
        return state.setValue(property, property.getValue(value).orElseThrow(() -> new IllegalArgumentException(value)));
    }

    // Exact decimal text (shortest round-trip form); parse with a correctly rounding reader.
    static <T extends Comparable<T>> String valueName(BlockState state, Property<T> property) {
        return property.getName(state.getValue(property));
    }

    static JsonArray vec(double... xs) { JsonArray a = new JsonArray(); for (double x : xs) a.add(Double.toString(x)); return a; }

    static final String VERSION_NAME = "1.16.1";
    static Iterable<Block> blocks() { return Registry.BLOCK; }
    static String blockName(Block block) { return Registry.BLOCK.getKey(block).toString(); }
    static boolean hasTag(Block block, net.minecraft.tags.Tag<Block> tag) { return block.is(tag); }

    // Block audit: per-state collision boxes and flags, per-block movement hooks.
    // `spec` lists hook=baseClass:method:parameterTypes in official names (from run.py).
    static JsonObject exportBlocks(String spec) throws Exception {
        Map<String, Method> hooks = new LinkedHashMap<>();
        for (String entry : spec.split(";")) {
            String[] kv = entry.split("="), p = kv[1].split(":");
            Method found = null;
            for (Method m : Class.forName(p[0]).getDeclaredMethods()) {
                String types = String.join(",", Arrays.stream(m.getParameterTypes()).map(Class::getName).toList());
                if (m.getName().equals(p[1]) && types.equals(p.length > 2 ? p[2] : "")) {
                    if (found != null) throw new IllegalStateException("ambiguous " + entry);
                    found = m;
                }
            }
            if (found == null) throw new IllegalStateException("missing " + entry);
            hooks.put(kv[0], found);
        }
        JsonArray shapes = new JsonArray();
        Map<String, Integer> shapeIndex = new HashMap<>();
        JsonArray stateShape = new JsonArray(), fluid = new JsonArray(), fluids = new JsonArray(), blocksMotion = new JsonArray(), sturdy = new JsonArray(), suffocating = new JsonArray(), positional = new JsonArray();
        BlockPos other = new BlockPos(7, 64, -13);
        int count = 0;
        for (int id = 0; ; id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            if (state == null) break;
            count++;
            // The context-taking form is what entity collision calls (never the cached shape).
            JsonArray boxes = boxes(state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, CollisionContext.empty()));
            if (!boxes.equals(boxes(state.getCollisionShape(EmptyBlockGetter.INSTANCE, other, CollisionContext.empty())))) positional.add(id);
            String key = boxes.toString();
            Integer index = shapeIndex.get(key);
            if (index == null) { index = shapes.size(); shapeIndex.put(key, index); shapes.add(boxes); }
            stateShape.add(index);
            FluidState fs = state.getFluidState();
            if (!fs.isEmpty()) {
                fluid.add(id);
                // [state, 1 water / 2 lava, amount, falling]
                boolean falling = fs.getProperties().contains(FlowingFluid.FALLING) && fs.getValue(FlowingFluid.FALLING);
                fluids.add(ints(id, fs.is(FluidTags.WATER) ? 1 : 2, fs.getAmount(), falling ? 1 : 0));
            }
            if (state.getMaterial().blocksMotion()) blocksMotion.add(id);
            int faces = 0;
            Direction[] horizontal = {Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST};
            for (int d = 0; d < 4; d++) if (state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, horizontal[d])) faces |= 1 << d;
            // Flow ignores the faces of ice (FlowingFluid.isSolidFace).
            if (state.getMaterial() == net.minecraft.world.level.material.Material.ICE) faces |= 16;
            if (faces != 0) sturdy.add(ints(id, faces));
            if (state.isSuffocating(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)) suffocating.add(id);
        }
        JsonArray blocks = new JsonArray();
        for (Block block : blocks()) {
            JsonObject b = new JsonObject();
            b.addProperty("name", blockName(block));
            b.addProperty("class", block.getClass().getName());
            b.addProperty("first_state", Block.getId(block.getStateDefinition().getPossibleStates().get(0)));
            b.addProperty("states", block.getStateDefinition().getPossibleStates().size());
            b.addProperty("friction", Float.toString(block.getFriction()));
            b.addProperty("speed_factor", Float.toString(block.getSpeedFactor()));
            b.addProperty("jump_factor", Float.toString(block.getJumpFactor()));
            JsonArray tags = new JsonArray();
            if (hasTag(block, BlockTags.CLIMBABLE)) tags.add("climbable");
            if (hasTag(block, BlockTags.FENCES)) tags.add("fences");
            if (hasTag(block, BlockTags.WALLS)) tags.add("walls");
            if (block instanceof FenceGateBlock) tags.add("fence_gate");
            if (block instanceof TrapDoorBlock) tags.add("trapdoor");
            b.add("tags", tags);
            JsonObject implemented = new JsonObject();
            for (Map.Entry<String, Method> hook : hooks.entrySet()) {
                Class<?> base = hook.getValue().getDeclaringClass();
                for (Class<?> c = block.getClass(); c != base && c != null; c = c.getSuperclass()) {
                    try {
                        c.getDeclaredMethod(hook.getValue().getName(), hook.getValue().getParameterTypes());
                        implemented.addProperty(hook.getKey(), c.getName());
                        break;
                    } catch (NoSuchMethodException ignored) {
                    }
                }
            }
            b.add("hooks", implemented);
            blocks.add(b);
        }
        JsonObject out = new JsonObject();
        out.addProperty("state_count", count);
        out.add("shapes", shapes);
        out.add("state_shapes", stateShape);
        out.add("fluid_states", fluid);
        out.add("fluids", fluids);
        out.add("blocks_motion_states", blocksMotion);
        out.add("sturdy_faces", sturdy);
        out.add("suffocating_states", suffocating);
        out.add("positional_shape_states", positional);
        JsonObject attributeIds = new JsonObject(), effectIds = new JsonObject();
        for (var a : Registry.ATTRIBUTE) attributeIds.addProperty(Registry.ATTRIBUTE.getKey(a).toString(), Registry.ATTRIBUTE.getId(a));
        for (var e : Registry.MOB_EFFECT) effectIds.addProperty(Registry.MOB_EFFECT.getKey(e).toString(), Registry.MOB_EFFECT.getId(e));
        out.add("attribute_ids", attributeIds);
        out.add("effect_ids", effectIds);
        out.add("blocks", blocks);
        return out;
    }
    static JsonArray ints(int... xs) { JsonArray a = new JsonArray(); for (int x : xs) a.add(x); return a; }
    static JsonArray boxes(VoxelShape shape) {
        JsonArray list = new JsonArray();
        for (AABB box : shape.toAabbs()) list.add(vec(box.minX, box.minY, box.minZ, box.maxX, box.maxY, box.maxZ));
        return list;
    }

    static final int CLEAR = 12;

    static JsonObject scenario(ServerLevel level, BlockPos origin, JsonObject scenario) {
        for (int cx = (origin.getX() - CLEAR) >> 4; cx <= (origin.getX() + CLEAR) >> 4; cx++)
            for (int cz = (origin.getZ() - CLEAR) >> 4; cz <= (origin.getZ() + CLEAR) >> 4; cz++)
                level.getChunk(cx, cz);
        BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (BlockPos p : BlockPos.betweenClosed(origin.offset(-CLEAR, -CLEAR, -CLEAR), origin.offset(CLEAR, CLEAR, CLEAR)))
            level.setBlock(p, air, 18);
        JsonObject resolved = new JsonObject();
        for (JsonElement e : scenario.getAsJsonArray("blocks")) {
            JsonArray b = e.getAsJsonArray();
            BlockState state = parseState(b.get(6).getAsString());
            JsonObject properties = new JsonObject();
            for (Property<?> property : state.getProperties()) properties.addProperty(property.getName(), valueName(state, property));
            resolved.add(b.get(6).getAsString(), properties);
            for (int x = b.get(0).getAsInt(); x <= b.get(3).getAsInt(); x++)
                for (int y = b.get(1).getAsInt(); y <= b.get(4).getAsInt(); y++)
                    for (int z = b.get(2).getAsInt(); z <= b.get(5).getAsInt(); z++)
                        level.setBlock(origin.offset(x, y, z), state, 18);
        }
        OraclePlayer player = new OraclePlayer(level);
        JsonArray start = scenario.getAsJsonArray("start");
        player.setPos(origin.getX() + start.get(0).getAsDouble(), origin.getY() + start.get(1).getAsDouble(), origin.getZ() + start.get(2).getAsDouble());
        player.setOnGround(scenario.has("on_ground") ? scenario.get("on_ground").getAsBoolean() : true);
        if (scenario.has("boat")) return boatScenario(level, origin, scenario, resolved, player);
        if (scenario.has("attributes")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("attributes").entrySet()) {
                var attribute = Registry.ATTRIBUTE.getOptional(new ResourceLocation(a.getKey())).orElseThrow();
                player.getAttribute(attribute).setBaseValue(a.getValue().getAsDouble());
            }
        }
        if (scenario.has("effects")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("effects").entrySet()) {
                var effect = Registry.MOB_EFFECT.getOptional(new ResourceLocation(a.getKey())).orElseThrow();
                player.addEffect(new MobEffectInstance(effect, 100000, a.getValue().getAsInt()));
            }
        }
        JsonObject initial = new JsonObject();
        var speed = player.getAttribute(net.minecraft.world.entity.ai.attributes.Attributes.MOVEMENT_SPEED);
        initial.addProperty("movement_speed_base", Double.toString(speed.getBaseValue()));
        JsonArray modifiers = new JsonArray();
        for (var m : speed.getModifiers()) {
            JsonObject o = new JsonObject();
            o.addProperty("id", m.getId().toString());
            o.addProperty("operation", m.getOperation().name());
            o.addProperty("amount", Double.toString(m.getAmount()));
            modifiers.add(o);
        }
        initial.add("movement_speed_modifiers", modifiers);
        initial.addProperty("food", player.getFoodData().getFoodLevel());
        JsonArray frames = new JsonArray();
        for (JsonElement e : scenario.getAsJsonArray("ticks")) {
            JsonObject t = e.getAsJsonObject();
            int forward = t.has("forward") ? t.get("forward").getAsInt() : 0;
            int strafe = t.has("strafe") ? t.get("strafe").getAsInt() : 0;
            Keys keys = new Keys();
            keys.up = forward > 0; keys.down = forward < 0; keys.left = strafe > 0; keys.right = strafe < 0;
            keys.jumping = t.has("jump") && t.get("jump").getAsBoolean();
            keys.shiftKeyDown = t.has("sneak") && t.get("sneak").getAsBoolean();
            keys.sprint = t.has("sprint") && t.get("sprint").getAsBoolean();
            player.next = keys;
            // Item use as the client starts it (Minecraft.handleKeybinds runs before the player tick).
            String using = t.has("using") ? t.get("using").getAsString() : null;
            if (using != null && !player.isUsingItem()) {
                player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND,
                    new net.minecraft.world.item.ItemStack(Registry.ITEM.get(new ResourceLocation(using))));
                player.startUsingItem(net.minecraft.world.InteractionHand.MAIN_HAND);
            } else if (using == null && player.isUsingItem()) {
                player.stopUsingItem();
            }
            player.yRot = t.get("yaw").getAsFloat();
            player.xRot = t.has("pitch") ? t.get("pitch").getAsFloat() : 0.0F;
            player.tick();
            Vec3 v = player.getDeltaMovement();
            JsonObject f = new JsonObject();
            f.add("position", vec(player.getX() - origin.getX(), player.getY() - origin.getY(), player.getZ() - origin.getZ()));
            f.add("velocity", vec(v.x, v.y, v.z));
            f.addProperty("on_ground", player.isOnGround());
            f.addProperty("horizontal_collision", player.horizontalCollision);
            f.addProperty("sprinting", player.isSprinting());
            f.addProperty("crouching", player.crouching);
            f.addProperty("pose", player.getPose().name());
            f.addProperty("in_water", player.isInWater());
            f.addProperty("swimming", player.isSwimming());
            f.addProperty("using", player.isUsingItem());
            f.addProperty("speed", Float.toString(player.getSpeed()));
            f.addProperty("fall_distance", Double.toString(player.fallDistance));
            frames.add(f);
        }
        JsonObject out = new JsonObject();
        out.addProperty("name", scenario.get("name").getAsString());
        out.add("initial", initial);
        out.add("states", resolved);
        out.add("frames", frames);
        return out;
    }

    // The unchanged native boat methods run inside the original ServerLevel.
    // Only status/input scheduling is supplied; no vehicle method is replaced.
    static JsonObject boatScenario(ServerLevel level, BlockPos origin, JsonObject scenario,
                                   JsonObject resolved, OraclePlayer player) {
        try {
            var boat = new net.minecraft.world.entity.vehicle.Boat(level, player.getX(), player.getY(), player.getZ());
            boat.setPos(player.getX(), player.getY(), player.getZ());
            boat.yRot = scenario.has("yaw") ? scenario.get("yaw").getAsFloat() : 0.0F;
            var nearby = level.getEntities(boat, boat.getBoundingBox().inflate(CLEAR));
            // stopRiding registers artificial players in the legacy chunk.
            // Retire fixtures from previous scenarios before collision queries.
            for (var entity : nearby) entity.remove();
            player.startRiding(boat, true);
            if (scenario.has("velocity")) {
                JsonArray v = scenario.getAsJsonArray("velocity");
                boat.setDeltaMovement(new Vec3(v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble()));
            }
            Class<?> base = net.minecraft.world.entity.vehicle.Boat.class;
            // Names from the pinned official mappings: reflection strings are
            // not changed when SpecialSource remaps the harness bytecode.
            Field status = base.getDeclaredField("aE"), old = base.getDeclaredField("aF"), angular = base.getDeclaredField("ar");
            for (Field f : List.of(status, old, angular)) f.setAccessible(true);
            Field[] input = new Field[4];
            String[] inputNames = {"ay", "az", "aA", "aB"};
            for (int i = 0; i < 4; i++) { input[i] = base.getDeclaredField(inputNames[i]); input[i].setAccessible(true); }
            Method get = base.getDeclaredMethod("s"), floating = base.getDeclaredMethod("v"), control = base.getDeclaredMethod("x");
            for (Method m : List.of(get, floating, control)) m.setAccessible(true);
            JsonArray frames = new JsonArray();
            for (JsonElement e : scenario.getAsJsonArray("ticks")) {
                JsonObject t = e.getAsJsonObject();
                int forward = t.has("forward") ? t.get("forward").getAsInt() : 0;
                int strafe = t.has("strafe") ? t.get("strafe").getAsInt() : 0;
                old.set(boat, status.get(boat));
                status.set(boat, get.invoke(boat));
                boolean[] keys = {strafe > 0, strafe < 0, forward > 0, forward < 0};
                for (int i = 0; i < 4; i++) input[i].setBoolean(boat, keys[i]);
                floating.invoke(boat);
                control.invoke(boat);
                boat.move(net.minecraft.world.entity.MoverType.SELF, boat.getDeltaMovement());
                Vec3 v = boat.getDeltaMovement();
                JsonObject f = new JsonObject();
                f.add("position", vec(boat.getX()-origin.getX(), boat.getY()-origin.getY(), boat.getZ()-origin.getZ()));
                f.add("velocity", vec(v.x, v.y, v.z));
                f.add("rotation", vec(boat.yRot, boat.xRot));
                f.addProperty("angular_velocity", Float.toString(angular.getFloat(boat)));
                f.addProperty("on_ground", boat.isOnGround());
                f.addProperty("in_water", status.get(boat).toString().equals("IN_WATER"));
                JsonArray paddles = new JsonArray();
                paddles.add(boat.getPaddleState(0)); paddles.add(boat.getPaddleState(1));
                f.add("paddles", paddles);
                frames.add(f);
            }
            player.stopRiding();
            player.remove();
            boat.remove();
            JsonObject out = new JsonObject();
            out.addProperty("name", scenario.get("name").getAsString());
            out.add("states", resolved); out.add("frames", frames);
            return out;
        } catch (Exception e) { throw new RuntimeException(e); }
    }

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable e) {
            e.printStackTrace();
            System.exit(1);
        }
    }

    static void run(String[] args) throws Exception {
        JsonArray scenarios = args[0].equals("--blocks") ? null : new JsonParser().parse(Files.readString(Path.of(args[0]))).getAsJsonArray();
        net.minecraft.server.Main.main(new String[]{"--nogui"});
        MinecraftServer server = null;
        for (int i = 0; i < 600 && (server == null || server.getTickCount() < 20); i++) {
            Thread.sleep(500);
            if (server == null) server = findServer();
        }
        if (server == null || server.getTickCount() < 20) throw new IllegalStateException("server did not start");
        if (args[0].equals("--blocks")) {
            JsonObject[] audit = new JsonObject[1];
            server.executeBlocking(() -> {
                try {
                    audit[0] = exportBlocks(System.getProperty("voxrig.hooks"));
                } catch (Throwable e) {
                    e.printStackTrace();
                    System.exit(1);
                }
            });
            audit[0].addProperty("version", VERSION_NAME);
            Files.writeString(Path.of(args[1]), new GsonBuilder().create().toJson(audit[0]) + "\n");
            System.out.println("ORACLE DONE blocks");
            server.halt(false);
            System.exit(0);
        }
        JsonArray results = new JsonArray();
        MinecraftServer s = server;
        server.executeBlocking(() -> {
            ServerLevel level = s.overworld();
            BlockPos origin = new BlockPos(1024, 100, 1024);
            try {
                for (JsonElement e : scenarios) results.add(scenario(level, origin, e.getAsJsonObject()));
            } catch (Throwable e) {
                e.printStackTrace();
                System.exit(1);
            }
        });
        JsonObject out = new JsonObject();
        out.addProperty("version", net.minecraft.SharedConstants.getCurrentVersion().getName());
        out.add("results", results);
        Files.writeString(Path.of(args[1]), new GsonBuilder().create().toJson(out) + "\n");
        System.out.println("ORACLE DONE " + results.size());
        server.halt(false);
        System.exit(0);
    }
}
