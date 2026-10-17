package jp.nlaocs.skript.selectionobserver;

import java.lang.instrument.Instrumentation;
import java.net.URLClassLoader;
import java.nio.file.Path;

public final class SelectionAgent {
    public static void agentmain(String jar, Instrumentation instrumentation) throws Exception {
        Class<?> skript = null;
        for (Class<?> type : instrumentation.getAllLoadedClasses()) {
            if (type.getName().equals("ch.njol.skript.Skript")) {
                if (skript != null) throw new IllegalStateException("Multiple Skript classes");
                skript = type;
            }
        }
        if (skript == null) throw new IllegalStateException("Skript is not loaded");
        var loader = new URLClassLoader(new java.net.URL[]{Path.of(jar).toUri().toURL()}, skript.getClassLoader());
        Class.forName("jp.nlaocs.skript.selectionobserver.SelectionBridge", true, loader).getMethod("start").invoke(null);
    }
}
