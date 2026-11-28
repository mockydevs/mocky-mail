buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
        // Kotlin half of rustls-platform-verifier (Android certificate checks).
        // A copy is kept in maven/ (from the rustls project's maven-archive branch,
        // checked against its published SHA-1), so builds don't depend on
        // GitHub's raw file server, which times out now and then. Update it
        // together with rustls-platform-verifier-android in Cargo.lock.
        maven {
            url = uri(rootProject.file("maven"))
            content { includeGroup("org.rustls") }
        }
    }
}

tasks.register("clean").configure {
    delete("build")
}

