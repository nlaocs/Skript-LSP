package codex;

import ch.njol.skript.ScriptLoader;
import ch.njol.skript.Skript;
import ch.njol.skript.config.Config;
import ch.njol.skript.config.Node;
import ch.njol.skript.config.SectionNode;
import ch.njol.skript.lang.Expression;
import ch.njol.skript.lang.ParseContext;
import ch.njol.skript.lang.SkriptEvent;
import ch.njol.skript.lang.SkriptParser;
import ch.njol.skript.lang.parser.ParserInstance;
import ch.njol.skript.log.RetainingLogHandler;
import com.google.gson.*;
import com.sun.net.httpserver.HttpServer;
import org.bukkit.Bukkit;
import org.skriptlang.skript.registration.DefaultSyntaxInfos;
import org.skriptlang.skript.registration.SyntaxInfo;
import org.skriptlang.skript.registration.SyntaxRegistry;
import org.skriptlang.skript.util.Priority;
import java.io.ByteArrayInputStream;
import java.lang.reflect.Modifier;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;

/** Isolated diagnostic fixture: supported registry API and parsing only; never execute an expression. */
public final class SelectionBridge {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static final List<DefaultSyntaxInfos.Expression<?, ?>> BASE = new ArrayList<>();
    private static final IdentityHashMap<SyntaxInfo<?>, String> LABELS = new IdentityHashMap<>();
    private static HttpServer http;
    private static ExecutorService executor;

    public static void start() throws Exception {
        http = HttpServer.create(new InetSocketAddress("127.0.0.1", Integer.getInteger("selection.port", 18124)), 0);
        executor = Executors.newSingleThreadExecutor(r -> { var t = new Thread(r, "selection-observer"); t.setDaemon(true); return t; });
        http.setExecutor(executor);
        for (String path : List.of("configure", "registry", "parse", "export")) {
            http.createContext("/" + path, exchange -> {
                int status = 200; Object result;
                try {
                    if (!exchange.getRequestMethod().equals("POST")) throw new IllegalArgumentException("POST required");
                    byte[] body = exchange.getRequestBody().readNBytes(65537);
                    if (body.length > 65536) throw new IllegalArgumentException("Request too large");
                    var request = body.length == 0 ? new JsonObject() : JsonParser.parseString(new String(body, StandardCharsets.UTF_8)).getAsJsonObject();
                    result = Bukkit.getScheduler().callSyncMethod(Skript.getInstance(), () -> dispatch(path, request)).get(150, TimeUnit.SECONDS);
                } catch (Throwable error) {
                    status = 500; result = Map.of("error", error.toString(), "cause", String.valueOf(error.getCause()));
                }
                byte[] bytes = JSON.toJson(result).getBytes(StandardCharsets.UTF_8);
                exchange.getResponseHeaders().set("Content-Type", "application/json; charset=utf-8");
                exchange.sendResponseHeaders(status, bytes.length);
                exchange.getResponseBody().write(bytes); exchange.close();
            });
        }
        http.createContext("/stop", exchange -> {
            exchange.sendResponseHeaders(200, 0); exchange.close(); http.stop(0); executor.shutdown();
        });
        http.start();
        System.out.println("SELECTION_OBSERVER_READY");
    }

    private static Object dispatch(String action, JsonObject request) throws Exception {
        if (!Bukkit.getOnlinePlayers().isEmpty()) throw new IllegalStateException("Refuse diagnostic with online players");
        return switch (action) {
            case "configure" -> configure(request);
            case "registry" -> registry();
            case "parse" -> parse(request);
            case "export" -> Map.of("commandAccepted", Bukkit.dispatchCommand(Bukkit.getConsoleSender(), "skgen"));
            default -> throw new IllegalArgumentException(action);
        };
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static Object configure(JsonObject request) throws Exception {
        if (!BASE.isEmpty()) throw new IllegalStateException("Configure only once per boot");
        var registry = Skript.instance().syntaxRegistry();
        var owner = Skript.getAddon(request.get("owner").getAsString());
        if (owner == null) throw new IllegalArgumentException("Unknown fixture owner");
        var registrations = owner.syntaxRegistry();
        BASE.addAll(registry.syntaxes(SyntaxRegistry.EXPRESSION));
        for (JsonElement element : request.getAsJsonArray("definitions")) {
            var row = element.getAsJsonObject();
            String type = row.get("class").getAsString();
            var matches = BASE.stream().filter(info -> info.type().getName().equals(type)).toList();
            if (matches.size() != 1) throw new IllegalArgumentException("Expected one base registration: " + type + " / " + matches.size());
            var builder = matches.getFirst().toBuilder().clearPatterns();
            for (JsonElement pattern : row.getAsJsonArray("patterns")) builder.addPattern(pattern.getAsString());
            Priority priority = switch (row.get("priority").getAsString()) {
                case "before" -> Priority.before(SyntaxInfo.SIMPLE);
                case "same" -> SyntaxInfo.SIMPLE;
                case "after" -> Priority.after(SyntaxInfo.SIMPLE);
                default -> throw new IllegalArgumentException("Unknown priority");
            };
            builder.priority(priority);
            var info = (DefaultSyntaxInfos.Expression<?, ?>) builder.build();
            registrations.register(SyntaxRegistry.EXPRESSION, info);
            LABELS.put(info, row.get("id").getAsString());
        }
        return registry();
    }

    private static Object registry() throws Exception {
        var result = new LinkedHashMap<String, Object>();
        var entries = new ArrayList<Object>(); int index = 0;
        for (var info : Skript.instance().syntaxRegistry().syntaxes(SyntaxRegistry.EXPRESSION)) {
            var row = new LinkedHashMap<String, Object>();
            row.put("registryIndex", index++); row.put("class", info.type().getName());
            row.put("returnType", info.returnType().getName()); row.put("patterns", new ArrayList<>(info.patterns()));
            row.put("priority", info.priority().toString()); row.put("origin", info.origin().toString());
            row.put("probeId", LABELS.get(info)); entries.add(row);
        }
        var plugins = new ArrayList<Object>();
        for (var plugin : Bukkit.getPluginManager().getPlugins()) {
            var jar = Path.of(plugin.getClass().getProtectionDomain().getCodeSource().getLocation().toURI());
            plugins.add(Map.of("name", plugin.getName(), "version", plugin.getDescription().getVersion(), "enabled", plugin.isEnabled(),
                "sha256", HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(jar)))));
        }
        result.put("expressions", entries); result.put("plugins", plugins);
        result.put("java", System.getProperty("java.version")); result.put("minecraft", Bukkit.getMinecraftVersion());
        result.put("onlinePlayers", Bukkit.getOnlinePlayers().size());
        return result;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static Object parse(JsonObject request) throws Exception {
        String input = request.get("input").getAsString();
        String eventName = request.get("event").getAsString();
        if (!Set.of("join", "quit").contains(eventName)) throw new IllegalArgumentException("Unsupported diagnostic Event");
        var parser = ParserInstance.get();
        parser.setInactive(); parser.setActive(ScriptLoader.createDummyScript("selection-diagnostic.sk", null));
        var log = new RetainingLogHandler().start();
        var result = new LinkedHashMap<String, Object>();
        result.put("input", input); result.put("event", eventName);
        result.put("parseFlags", "PARSE_EXPRESSIONS"); result.put("parseContext", "DEFAULT");
        try {
            var config = new Config(new ByteArrayInputStream(("on " + eventName + ":\n\t# diagnostic\n").getBytes(StandardCharsets.UTF_8)), "selection-diagnostic.sk", true, false, ":");
            SectionNode section = null;
            for (Node node : config.getMainNode()) if (node instanceof SectionNode found) section = found;
            var event = SkriptEvent.parse(eventName, section, null);
            if (event == null) throw new IllegalStateException("Event did not parse");
            parser.setCurrentStructure(event); parser.setCurrentEvent(eventName, event.getEventClasses());
            result.put("eventClasses", Arrays.stream(event.getEventClasses()).map(Class::getName).toList());
            var types = new ArrayList<Class<?>>();
            for (var value : request.getAsJsonArray("expectedTypes")) {
                String name = value.getAsString(); Class<?> found = null;
                for (var plugin : Bukkit.getPluginManager().getPlugins()) {
                    try { found = Class.forName(name, false, plugin.getClass().getClassLoader()); break; }
                    catch (ClassNotFoundException ignored) { }
                }
                if (found == null) throw new ClassNotFoundException(name);
                types.add(found);
            }
            result.put("expectedTypes", types.stream().map(Class::getName).toList());
            var expression = new SkriptParser(input, SkriptParser.PARSE_EXPRESSIONS, ParseContext.DEFAULT).parseExpression(types.toArray(Class[]::new));
            result.put("selected", expression == null ? null : expression(expression));
            result.put("errors", log.getErrors().stream().map(Object::toString).toList());
            result.put("parseOnly", "SkriptParser.parseExpression; no validateScript/postLoad/execute");
            return result;
        } finally {
            log.stop(); parser.deleteCurrentEvent(); parser.setCurrentStructure(null); parser.setInactive();
        }
    }

    private static Object expression(Expression<?> value) throws Exception {
        var result = new LinkedHashMap<String, Object>();
        result.put("class", value.getClass().getName()); result.put("returnType", value.getReturnType().getName());
        result.put("single", value.isSingle());
        var source = value.getSource();
        if (source != value) result.put("conversionSource", expression(source));
        var scalars = new LinkedHashMap<String, Object>();
        for (Class<?> type = value.getClass(); type != null && type != Object.class; type = type.getSuperclass()) {
            for (var field : type.getDeclaredFields()) {
                if (Modifier.isStatic(field.getModifiers()) || !field.trySetAccessible()) continue;
                Object fieldValue = field.get(value);
                if (fieldValue instanceof String || fieldValue instanceof Number || fieldValue instanceof Boolean) scalars.put(type.getName() + "." + field.getName(), fieldValue);
            }
        }
        result.put("observedScalarFields", scalars);
        result.put("matchedPattern", null);
        result.put("matchedPatternEvidence", "not retained by these upstream fixture instances; map only unique exact registered literal patterns separately");
        return result;
    }
}
