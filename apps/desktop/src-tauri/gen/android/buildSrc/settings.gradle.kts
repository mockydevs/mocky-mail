// Where Gradle finds the plugins that build buildSrc itself (kotlin-dsl
// pulls in the Kotlin Gradle plugin, which is on Maven Central only).
pluginManagement {
    repositories {
        gradlePluginPortal()
        google()
        mavenCentral()
    }
}
