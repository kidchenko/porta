//! Which technology is behind a port: Next.js, Django, Postgres, plain Node.
//! It picks the row's logo, and names the stack in the port's submenu.
//!
//! Read off the process, most telling first:
//!
//! 1. a framework or tool on its command line: `node .../next/dist/bin/next
//!    dev` is Next.js, `python manage.py runserver` is Django;
//! 2. a server known by its executable: `postgres`, `redis-server`, `nginx`;
//! 3. the runtime itself: `node`, `python3.12`, `ruby`, a `go run` binary.

use crate::ports::Listener;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tech {
    /// The logo's file in assets/icons/tech, without `.png`.
    pub key: &'static str,
    /// How people write its name.
    pub label: &'static str,
}

const fn t(key: &'static str, label: &'static str) -> Tech {
    Tech { key, label }
}

/// Words on a command line (paths split into their parts) that give away the
/// framework or tool. Checked in order, so specific ones come before the
/// general ones they run on (Nuxt before Vite, Storybook before Webpack).
const ARGS: &[(&[&str], Tech)] = &[
    (&["next", "next-server"], t("nextjs", "Next.js")),
    (&["nuxt", "nuxi"], t("nuxtjs", "Nuxt")),
    (&["astro"], t("astro", "Astro")),
    (
        &["remix", "@remix-run", "react-router"],
        t("remix", "Remix"),
    ),
    (&["gatsby"], t("gatsby", "Gatsby")),
    (&["@sveltejs", "svelte-kit"], t("svelte", "SvelteKit")),
    (&["@angular", "ng"], t("angular", "Angular")),
    (&["@vue", "vue-cli-service"], t("vuejs", "Vue")),
    (&["react-scripts"], t("react", "React")),
    (&["solid-start", "@solidjs"], t("solidjs", "SolidStart")),
    (&["@builder.io", "qwik"], t("qwik", "Qwik")),
    (&["ember", "ember-cli"], t("ember", "Ember")),
    (&["expo", "@expo"], t("expo", "Expo")),
    (
        &["storybook", "@storybook", "start-storybook"],
        t("storybook", "Storybook"),
    ),
    (&["vitest"], t("vitest", "Vitest")),
    (&["vite"], t("vitejs", "Vite")),
    (&["webpack", "webpack-dev-server"], t("webpack", "webpack")),
    (&["rollup"], t("rollup", "Rollup")),
    (&["@nestjs", "nest"], t("nestjs", "NestJS")),
    (&["fastify", "fastify-cli"], t("fastify", "Fastify")),
    (&["express"], t("express", "Express")),
    (&["electron"], t("electron", "Electron")),
    (&["prisma"], t("prisma", "Prisma")),
    (&["@apollo"], t("apollographql", "Apollo")),
    (
        &["wrangler", "workerd"],
        t("cloudflare", "Cloudflare Workers"),
    ),
    (&["vercel"], t("vercel", "Vercel")),
    (&["netlify", "netlify-cli"], t("netlify", "Netlify")),
    (&["firebase", "firebase-tools"], t("firebase", "Firebase")),
    (&["supabase"], t("supabase", "Supabase")),
    // Python
    (&["manage.py", "django", "daphne"], t("django", "Django")),
    (&["flask"], t("flask", "Flask")),
    (&["uvicorn", "fastapi"], t("fastapi", "FastAPI")),
    (&["streamlit"], t("streamlit", "Streamlit")),
    (
        &[
            "jupyter",
            "jupyter-lab",
            "jupyter-notebook",
            "jupyterlab",
            "ipykernel_launcher",
        ],
        t("jupyter", "Jupyter"),
    ),
    // Ruby, PHP, Elixir, JVM
    (&["rails", "puma"], t("rails", "Rails")),
    (&["jekyll"], t("jekyll", "Jekyll")),
    (&["artisan"], t("laravel", "Laravel")),
    (&["symfony"], t("symfony", "Symfony")),
    (&["phx.server", "phoenix"], t("phoenix", "Phoenix")),
    (&["bootrun", "spring-boot:run"], t("spring", "Spring Boot")),
    (
        &["gradle", "gradlew", "gradledaemon"],
        t("gradle", "Gradle"),
    ),
    (&["mvn", "mvnw", "maven"], t("maven", "Maven")),
    (
        &["catalina", "org.apache.catalina.startup.bootstrap"],
        t("tomcat", "Tomcat"),
    ),
    // Containers and tunnels
    (
        &["port-forward", "kubectl", "minikube"],
        t("kubernetes", "Kubernetes"),
    ),
    (&["ngrok"], t("ngrok", "ngrok")),
];

/// Substrings of a JVM or BEAM command line: those servers all run as `java`
/// or `beam.smp`, and say which they are in a class or app name.
const VMS: &[&str] = &["java", "javaw", "beam.smp", "beam", "erl"];
const INSIDE: &[(&str, Tech)] = &[
    ("org.springframework", t("spring", "Spring Boot")),
    ("org.elasticsearch", t("elasticsearch", "Elasticsearch")),
    ("kafka.kafka", t("apachekafka", "Kafka")),
    ("org.apache.cassandra", t("cassandra", "Cassandra")),
    ("org.neo4j", t("neo4j", "Neo4j")),
    ("jenkins.war", t("jenkins", "Jenkins")),
    ("rabbit", t("rabbitmq", "RabbitMQ")),
    ("couchdb", t("couchdb", "CouchDB")),
];

/// Executables, lower-case without `.exe`, that are a server or runtime in
/// their own right.
const EXES: &[(&[&str], Tech)] = &[
    (&["postgres", "postmaster"], t("postgresql", "PostgreSQL")),
    (&["mysqld"], t("mysql", "MySQL")),
    (&["mariadbd"], t("mariadb", "MariaDB")),
    (&["mongod", "mongos"], t("mongodb", "MongoDB")),
    (&["redis-server", "valkey-server"], t("redis", "Redis")),
    (&["memcached"], t("memcached", "Memcached")),
    (
        &["clickhouse", "clickhouse-server"],
        t("clickhouse", "ClickHouse"),
    ),
    (&["nginx"], t("nginx", "nginx")),
    (&["httpd", "apache2"], t("apache", "Apache")),
    (&["grafana", "grafana-server"], t("grafana", "Grafana")),
    (&["prometheus"], t("prometheus", "Prometheus")),
    (&["vault"], t("vault", "Vault")),
    (&["consul"], t("consul", "Consul")),
    (&["hugo"], t("hugo", "Hugo")),
    (&["ngrok"], t("ngrok", "ngrok")),
    (&["kubectl", "minikube"], t("kubernetes", "Kubernetes")),
    (
        &[
            "docker",
            "dockerd",
            "docker-proxy",
            "com.docker.backend",
            "com.docker.vpnkit",
            "vpnkit",
            "colima",
            "orbstack helper",
        ],
        t("docker", "Docker"),
    ),
    (&["podman", "gvproxy"], t("podman", "Podman")),
    (&["workerd"], t("cloudflare", "Cloudflare Workers")),
    (&["flutter", "flutter_tester"], t("flutter", "Flutter")),
    (&["dart", "dartaotruntime"], t("dart", "Dart")),
    // Runtimes last: the frameworks above say more.
    (&["node", "nodejs"], t("nodejs", "Node.js")),
    (&["deno"], t("denojs", "Deno")),
    (&["bun"], t("bun", "Bun")),
    (
        &[
            "python",
            "python3",
            "python2",
            "pythonw",
            "py",
            "uv",
            "gunicorn",
            "hypercorn",
        ],
        t("python", "Python"),
    ),
    (&["ruby", "rackup", "bundle"], t("ruby", "Ruby")),
    (&["php", "php-fpm", "frankenphp"], t("php", "PHP")),
    (&["java", "javaw"], t("java", "Java")),
    (&["kotlin"], t("kotlin", "Kotlin")),
    (&["scala", "sbt"], t("scala", "Scala")),
    (&["dotnet"], t("dotnetcore", ".NET")),
    (&["beam.smp", "erl", "beam"], t("erlang", "Erlang")),
    (&["elixir", "iex", "mix"], t("elixir", "Elixir")),
    (&["swift", "swift-run"], t("swift", "Swift")),
    (
        &["ghc", "runghc", "stack", "cabal"],
        t("haskell", "Haskell"),
    ),
    (
        &["perl", "plackup", "morbo", "hypnotoad"],
        t("perl", "Perl"),
    ),
    (&["lua", "luajit", "openresty"], t("lua", "Lua")),
    (&["clojure", "clj", "lein"], t("clojure", "Clojure")),
    (&["julia"], t("julia", "Julia")),
    (&["r", "rscript", "rsession"], t("r", "R")),
    (&["crystal"], t("crystal", "Crystal")),
    (&["zig"], t("zig", "Zig")),
    (&["ocaml", "dune"], t("ocaml", "OCaml")),
    (&["cargo"], t("rust", "Rust")),
    (&["go"], t("go", "Go")),
];

/// The technology behind `l`, where Porta knows its logo.
pub fn of(l: &Listener) -> Option<Tech> {
    detect(&l.name, &l.command, &l.path)
}

fn detect(name: &str, command: &str, path: &str) -> Option<Tech> {
    let exe = exe_key(name);
    let lower = command.to_ascii_lowercase();

    // Words on the command line, each path cut into its parts:
    // "/app/node_modules/next/dist/bin/next" → app, node_modules, next, ...
    // The first word too: a process may have renamed itself
    // ("next-server (v16.1.5)").
    let args: Vec<&str> = lower
        .split_whitespace()
        .flat_map(|w| w.trim_matches(['"', '\'']).split(['/', '\\']))
        .filter(|w| !w.is_empty())
        .collect();
    let bare = |w: &str| {
        let w = w.strip_suffix(".exe").unwrap_or(w);
        [".js", ".mjs", ".cjs", ".ts"]
            .iter()
            .find_map(|ext| w.strip_suffix(ext))
            .unwrap_or(w)
            .to_string()
    };
    let words: Vec<String> = args.iter().map(|w| bare(w)).collect();
    for (keys, tech) in ARGS {
        if keys.iter().any(|k| words.iter().any(|w| w == k)) {
            return Some(*tech);
        }
    }
    if VMS.contains(&exe.as_str()) {
        if let Some((_, tech)) = INSIDE.iter().find(|(needle, _)| lower.contains(needle)) {
            return Some(*tech);
        }
    }
    if let Some(tech) = by_exe(&exe) {
        return Some(tech);
    }

    // Binaries named after their project: `go run` builds into go-build
    // temporaries, `cargo run` into target/debug.
    let p = if path.is_empty() {
        lower.split_whitespace().next().unwrap_or("").to_string()
    } else {
        path.to_ascii_lowercase()
    }
    .replace('\\', "/");
    if p.contains("/go-build") {
        return Some(t("go", "Go"));
    }
    if p.contains("/target/debug/") || p.contains("/target/release/") {
        return Some(t("rust", "Rust"));
    }
    None
}

fn by_exe(exe: &str) -> Option<Tech> {
    if let Some((_, tech)) = EXES.iter().find(|(keys, _)| keys.contains(&exe)) {
        return Some(*tech);
    }
    // Versioned names: python3.12, ruby3.3, php8.3, node22, postgres16.
    let stem = exe.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-');
    (stem != exe && !stem.is_empty())
        .then(|| by_exe(stem))
        .flatten()
}

fn exe_key(name: &str) -> String {
    let n = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, command: &str) -> Option<&'static str> {
        detect(name, command, "").map(|t| t.key)
    }

    #[test]
    fn frameworks_win_over_their_runtime() {
        assert_eq!(
            key("node", "node /app/node_modules/next/dist/bin/next dev"),
            Some("nextjs")
        );
        assert_eq!(
            key("node", "node /app/node_modules/.bin/vite"),
            Some("vitejs")
        );
        assert_eq!(
            key("node", "node /app/node_modules/nuxi/bin/nuxi.mjs dev"),
            Some("nuxtjs")
        );
        assert_eq!(key("node", "node server.js"), Some("nodejs"));
        assert_eq!(key("node", "next-server (v16.1.5)"), Some("nextjs"));
        assert_eq!(
            key("Python", "/usr/bin/python3 manage.py runserver"),
            Some("django")
        );
        assert_eq!(
            key("python3.12", "python3.12 -m http.server"),
            Some("python")
        );
        assert_eq!(
            key("python3", ".venv/bin/python3 .venv/bin/uvicorn main:app"),
            Some("fastapi")
        );
        assert_eq!(key("ruby", "ruby bin/rails server"), Some("rails"));
        assert_eq!(key("php", "php artisan serve"), Some("laravel"));
    }

    #[test]
    fn servers_by_executable_or_class() {
        assert_eq!(
            key(
                "postgres",
                "/opt/homebrew/opt/postgresql@16/bin/postgres -D x"
            ),
            Some("postgresql")
        );
        assert_eq!(key("redis-server", "redis-server *:6379"), Some("redis"));
        assert_eq!(key("node.exe", ""), Some("nodejs"));
        assert_eq!(
            key(
                "java",
                "java -cp x org.springframework.boot.loader.JarLauncher"
            ),
            Some("spring")
        );
        assert_eq!(key("com.docker.backend", ""), Some("docker"));
    }

    #[test]
    fn built_binaries() {
        assert_eq!(
            detect("main", "/var/folders/x/go-build123/b001/exe/main", "").map(|t| t.key),
            Some("go")
        );
        assert_eq!(
            detect("api", "", "/Users/me/api/target/debug/api").map(|t| t.key),
            Some("rust")
        );
    }

    #[test]
    fn unknown_stays_unknown() {
        assert_eq!(
            key(
                "Spotify",
                "/Applications/Spotify.app/Contents/MacOS/Spotify"
            ),
            None
        );
        // A word inside a longer one is not a match.
        assert_eq!(key("myserver", "myserver --nextgen"), None);
    }

    #[test]
    fn every_logo_exists() {
        for tech in ARGS
            .iter()
            .map(|(_, t)| t)
            .chain(INSIDE.iter().map(|(_, t)| t))
            .chain(EXES.iter().map(|(_, t)| t))
        {
            assert!(
                crate::art::tech_png(tech.key).is_some(),
                "no logo for {}",
                tech.key
            );
        }
    }
}
