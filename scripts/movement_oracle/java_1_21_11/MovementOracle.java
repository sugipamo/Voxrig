// Movement oracle for Java 1.21.11. Written against Mojang's published names,
// compiled against a locally remapped copy of the official server, then remapped
// back so it runs inside the unchanged official server JAR (see run.py).
// No game code is copied: the player below calls the official movement code
// and only ports the client-side input handling of LocalPlayer, noted inline.
package voxrig.oracle;

import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.Field;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.Mth;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

public final class MovementOracle {
    /** A survival player driven like LocalPlayer, inside a server level. */
    static final class OraclePlayer extends Player {
        Input keys = Input.EMPTY;
        Input next = Input.EMPTY;
        Vec2 moveVector = Vec2.ZERO;
        boolean crouching;

        OraclePlayer(Level level) {
            super(level, new GameProfile(new UUID(0, 1), "oracle"));
            getAbilities().invulnerable = true;
        }

        @Override public GameType gameMode() { return GameType.SURVIVAL; }
        // LocalPlayer: the local instance is authoritative for its own movement.
        @Override public boolean isLocalPlayer() { return true; }
        @Override public boolean isClientAuthoritative() { return false; }
        @Override public boolean isShiftKeyDown() { return keys.shift(); }
        @Override public boolean isCrouching() { return crouching; }
        @Override public boolean isUnderWater() { return wasUnderwater; }
        boolean isMovingSlowly() { return isCrouching() || isVisuallyCrawling(); }

        static float impulse(boolean positive, boolean negative) {
            return positive == negative ? 0.0F : positive ? 1.0F : -1.0F;
        }

        // Port of the movement-relevant part of LocalPlayer.aiStep. Sprinting is
        // requested only by the sprint key (no double-tap), flying is not used.
        @Override public void aiStep() {
            Abilities abilities = getAbilities();
            crouching = !abilities.flying && !isSwimming() && !isPassenger()
                && canPlayerFitWithinBlocksAndEntitiesWhen(Pose.CROUCHING)
                && (isShiftKeyDown() || !isSleeping() && !canPlayerFitWithinBlocksAndEntitiesWhen(Pose.STANDING));
            keys = next;
            moveVector = new Vec2(impulse(keys.left(), keys.right()), impulse(keys.forward(), keys.backward())).normalized();
            if (!noPhysics) {
                double w = getBbWidth() * 0.35;
                moveTowardsClosestSpace(getX() - w, getZ() + w);
                moveTowardsClosestSpace(getX() - w, getZ() - w);
                moveTowardsClosestSpace(getX() + w, getZ() - w);
                moveTowardsClosestSpace(getX() + w, getZ() + w);
            }
            if (canStartSprinting() && keys.sprint()) setSprinting(true);
            if (isSprinting()) {
                if (isSwimming()) {
                    if (shouldStopSwimSprinting()) setSprinting(false);
                } else if (shouldStopRunSprinting()) {
                    setSprinting(false);
                }
            }
            if (isInWater() && keys.shift() && isAffectedByFluids()) goDownInWater();
            super.aiStep();
        }

        boolean hasForwardImpulse() { return moveVector.y > 1.0E-5F; }
        boolean isSprintingPossible(boolean flying) {
            return !isMobilityRestricted() && hasEnoughFoodToDoExhaustiveManoeuvres() && (flying || !isInShallowWater());
        }
        boolean canStartSprinting() {
            return !isSprinting() && hasForwardImpulse() && isSprintingPossible(getAbilities().flying)
                && !isUsingItem() && (!isFallFlying() || isUnderWater()) && (!isMovingSlowly() || isUnderWater());
        }
        boolean shouldStopRunSprinting() {
            return !isSprintingPossible(getAbilities().flying) || !hasForwardImpulse()
                || horizontalCollision && !minorHorizontalCollision;
        }
        boolean shouldStopSwimSprinting() {
            return !isSprintingPossible(true) || !isInWater() || !hasForwardImpulse() && !onGround() && !keys.shift();
        }

        void moveTowardsClosestSpace(double x, double z) {
            BlockPos pos = BlockPos.containing(x, getY(), z);
            if (!suffocatesAt(pos)) return;
            double dx = x - pos.getX(), dz = z - pos.getZ();
            Direction best = null;
            double nearest = Double.MAX_VALUE;
            for (Direction d : new Direction[]{Direction.WEST, Direction.EAST, Direction.NORTH, Direction.SOUTH}) {
                double along = d.getAxis().choose(dx, 0.0, dz);
                double distance = d.getAxisDirection() == Direction.AxisDirection.POSITIVE ? 1.0 - along : along;
                if (distance < nearest && !suffocatesAt(pos.relative(d))) { nearest = distance; best = d; }
            }
            if (best != null) {
                Vec3 v = getDeltaMovement();
                if (best.getAxis() == Direction.Axis.X) setDeltaMovement(0.1 * best.getStepX(), v.y, v.z);
                else setDeltaMovement(v.x, v.y, 0.1 * best.getStepZ());
            }
        }
        boolean suffocatesAt(BlockPos pos) {
            AABB box = getBoundingBox();
            AABB column = new AABB(pos.getX(), box.minY, pos.getZ(), pos.getX() + 1.0, box.maxY, pos.getZ() + 1.0).deflate(1.0E-7);
            return level().collidesWithSuffocatingBlock(this, column);
        }

        // LocalPlayer.applyInput / modifyInput: the client scales the key vector.
        @Override public void applyInput() {
            Vec2 v = modifyInput(moveVector);
            xxa = v.x;
            zza = v.y;
            jumping = keys.jump();
        }
        Vec2 modifyInput(Vec2 v) {
            if (v.lengthSquared() == 0.0F) return v;
            Vec2 scaled = v.scale(0.98F);
            if (isMovingSlowly()) scaled = scaled.scale((float) getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SNEAKING_SPEED));
            float length = scaled.length();
            if (length <= 0.0F) return scaled;
            Vec2 unit = scaled.scale(1.0F / length);
            float ax = Math.abs(unit.x), ay = Math.abs(unit.y);
            float ratio = ay > ax ? ax / ay : ay / ax;
            return unit.scale(Math.min(length * Mth.sqrt(1.0F + Mth.square(ratio)), 1.0F));
        }

        // LocalPlayer.isHorizontalCollisionMinor (Entity's default is false).
        @Override protected boolean isHorizontalCollisionMinor(Vec3 movement) {
            float yaw = getYRot() * (float) (Math.PI / 180.0);
            double s = Mth.sin(yaw), c = Mth.cos(yaw);
            double x = xxa * c - zza * s, z = zza * c + xxa * s;
            double input = Mth.square(x) + Mth.square(z), moved = Mth.square(movement.x) + Mth.square(movement.z);
            if (input < 1.0E-5F || moved < 1.0E-5F) return false;
            return Math.acos((x * movement.x + z * movement.z) / Math.sqrt(input * moved)) < 0.13962634F;
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
        Block block = BuiltInRegistries.BLOCK.getOptional(Identifier.parse(name)).orElseThrow(() -> new IllegalArgumentException(name));
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
    static JsonArray vec(double... xs) { JsonArray a = new JsonArray(); for (double x : xs) a.add(Double.toString(x)); return a; }

    static final int CLEAR = 12;

    static JsonObject run(ServerLevel level, BlockPos origin, JsonObject scenario) {
        for (int cx = (origin.getX() - CLEAR) >> 4; cx <= (origin.getX() + CLEAR) >> 4; cx++)
            for (int cz = (origin.getZ() - CLEAR) >> 4; cz <= (origin.getZ() + CLEAR) >> 4; cz++)
                level.getChunk(cx, cz);
        BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (BlockPos p : BlockPos.betweenClosed(origin.offset(-CLEAR, -CLEAR, -CLEAR), origin.offset(CLEAR, CLEAR, CLEAR)))
            level.setBlock(p, air, 18);
        for (JsonElement e : scenario.getAsJsonArray("blocks")) {
            JsonArray b = e.getAsJsonArray();
            BlockState state = parseState(b.get(6).getAsString());
            for (int x = b.get(0).getAsInt(); x <= b.get(3).getAsInt(); x++)
                for (int y = b.get(1).getAsInt(); y <= b.get(4).getAsInt(); y++)
                    for (int z = b.get(2).getAsInt(); z <= b.get(5).getAsInt(); z++)
                        level.setBlock(origin.offset(x, y, z), state, 18);
        }
        OraclePlayer player = new OraclePlayer(level);
        JsonArray start = scenario.getAsJsonArray("start");
        player.setPos(origin.getX() + start.get(0).getAsDouble(), origin.getY() + start.get(1).getAsDouble(), origin.getZ() + start.get(2).getAsDouble());
        player.setOnGround(scenario.has("on_ground") ? scenario.get("on_ground").getAsBoolean() : true);
        if (scenario.has("attributes")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("attributes").entrySet()) {
                var holder = BuiltInRegistries.ATTRIBUTE.get(Identifier.parse(a.getKey())).orElseThrow();
                player.getAttribute(holder).setBaseValue(a.getValue().getAsDouble());
            }
        }
        if (scenario.has("effects")) {
            for (Map.Entry<String, JsonElement> a : scenario.getAsJsonObject("effects").entrySet()) {
                var holder = BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(a.getKey())).orElseThrow();
                player.addEffect(new MobEffectInstance(holder, 100000, a.getValue().getAsInt()));
            }
        }
        JsonArray frames = new JsonArray();
        for (JsonElement e : scenario.getAsJsonArray("ticks")) {
            JsonObject t = e.getAsJsonObject();
            int forward = t.has("forward") ? t.get("forward").getAsInt() : 0;
            int strafe = t.has("strafe") ? t.get("strafe").getAsInt() : 0;
            player.next = new Input(forward > 0, forward < 0, strafe > 0, strafe < 0,
                t.has("jump") && t.get("jump").getAsBoolean(), t.has("sneak") && t.get("sneak").getAsBoolean(),
                t.has("sprint") && t.get("sprint").getAsBoolean());
            player.setYRot(t.get("yaw").getAsFloat());
            player.setXRot(t.has("pitch") ? t.get("pitch").getAsFloat() : 0.0F);
            player.tick();
            Vec3 v = player.getDeltaMovement();
            JsonObject f = new JsonObject();
            f.add("position", vec(player.getX() - origin.getX(), player.getY() - origin.getY(), player.getZ() - origin.getZ()));
            f.add("velocity", vec(v.x, v.y, v.z));
            f.addProperty("on_ground", player.onGround());
            f.addProperty("horizontal_collision", player.horizontalCollision);
            f.addProperty("sprinting", player.isSprinting());
            f.addProperty("crouching", player.crouching);
            f.addProperty("pose", player.getPose().name());
            f.addProperty("in_water", player.isInWater());
            frames.add(f);
        }
        JsonObject out = new JsonObject();
        out.addProperty("name", scenario.get("name").getAsString());
        out.add("frames", frames);
        return out;
    }

    public static void main(String[] args) throws Exception {
        JsonArray scenarios = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        net.minecraft.server.Main.main(new String[]{"--nogui"});
        MinecraftServer server = null;
        for (int i = 0; i < 600 && (server == null || server.getTickCount() < 20); i++) {
            Thread.sleep(500);
            if (server == null) server = findServer();
        }
        if (server == null || server.getTickCount() < 20) throw new IllegalStateException("server did not start");
        JsonArray results = new JsonArray();
        MinecraftServer s = server;
        server.executeBlocking(() -> {
            ServerLevel level = s.overworld();
            BlockPos origin = new BlockPos(1024, 100, 1024);
            for (JsonElement e : scenarios) results.add(run(level, origin, e.getAsJsonObject()));
        });
        JsonObject out = new JsonObject();
        out.addProperty("version", net.minecraft.SharedConstants.getCurrentVersion().name());
        out.add("results", results);
        Files.writeString(Path.of(args[1]), new GsonBuilder().create().toJson(out) + "\n");
        System.out.println("ORACLE DONE " + results.size());
        server.halt(false);
        System.exit(0);
    }
}
