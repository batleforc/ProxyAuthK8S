import Link from 'next/link';
import {
  ArrowRight,
  BookOpenText,
  Boxes,
  ExternalLink,
  GitBranch,
  ShieldCheck,
  Terminal,
  Merge,
  ArrowBigUp,
} from 'lucide-react';

const featureCards = [
  {
    title: 'Sécurité Renforcée',
    description:
      'Authentification centralisée avec possibilité de granularisation des permissions pour chaque cluster Kubernetes.',
    icon: ShieldCheck,
  },
  {
    title: 'CLI + kubectl Plugin',
    description:
      'Utilisez le CLI dédié et l\'intégration kubectl pour authentifier facilité la gestion des clusters.',
    icon: Terminal,
  },
  {
    title: 'Api Documenter',
    description:
      'Explore generated OpenAPI endpoints and behavior details directly inside the docs website.',
    icon: BookOpenText,
  },{
    title: "Interface Unifiée",
    description: "Une seule interface pour accéder à tous vos clusters. Fini de jongler avec plusieurs Kubeconfig incompréhensible. Intégration possible avec d'autre portails.",
    icon: Merge
  },{
    title: "Accès Instantané",
    description: "Redirection automatique vers les APIs avec authentification SSO. Pas d'impersonate pour sécuriser et partitionner les accès.",
    icon: ArrowBigUp
  },{
    title: "Open Source",
    description: "Code source disponible sur Github. Contribuez, signalez des problèmes ou personnalisez selon vos besoins.",
    icon: Boxes
  }
] as const;

const startSteps = [
  'git clone https://github.com/batleforc/proxyAuthK8s.git',
  'cd proxyAuthK8s',
  'yarn install',
  'yarn docs:dev',
] as const;

export default function HomePage() {
  return (
    <main className="mx-auto flex w-full max-w-6xl flex-1 flex-col gap-14 px-4 py-12 sm:px-6 lg:py-20">
      <section className="relative overflow-hidden rounded-2xl border bg-fd-card p-6 sm:p-10 lg:p-12">
        <div className="absolute -right-28 -top-28 h-64 w-64 rounded-full bg-fd-primary/10 blur-3xl" aria-hidden />
        <div className="absolute -left-20 -bottom-24 h-56 w-56 rounded-full bg-fd-secondary/60 blur-3xl" aria-hidden />

        <div className="relative z-10 flex flex-col gap-7">
          <p className="inline-flex w-fit items-center gap-2 rounded-full border px-3 py-1 text-xs font-semibold uppercase tracking-wide text-fd-muted-foreground">
            <Boxes className="size-3.5" />
            ProxyAuthK8s Documentation
          </p>

          <div className="max-w-3xl space-y-3">
            <h1 className="text-balance text-3xl font-bold tracking-tight sm:text-4xl lg:text-5xl">
              Votre passerelle sécurisée vers Kubernetes.
            </h1>
            <p className="text-pretty text-sm text-fd-muted-foreground sm:text-base lg:text-lg">
              Centralisez et sécurisez l&apos;accès à vos APIs Kubernetes. Une interface unique pour faciliter l&apos;accés a vos différents cluster.
            </p>
          </div>

          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <Link
              href="/docs"
              className="inline-flex items-center justify-center gap-2 rounded-md bg-fd-primary px-4 py-2 text-sm font-semibold text-fd-primary-foreground transition-colors hover:bg-fd-primary/85"
            >
              Vers la documentation
              <ArrowRight className="size-4" />
            </Link>
            <Link
              href="https://github.com/batleforc/proxyAuthK8s"
              className="inline-flex items-center justify-center gap-2 rounded-md border px-4 py-2 text-sm font-semibold transition-colors hover:bg-fd-accent"
            >
              Github
              <ExternalLink className="size-4" />
            </Link>
          </div>
        </div>
      </section>

      <section className="grid gap-4 md:grid-cols-3">
        {featureCards.map((item) => {
          const Icon = item.icon;
          return (
            <article key={item.title} className="rounded-xl border bg-fd-card p-5">
              <div className="mb-4 inline-flex rounded-md border bg-fd-background p-2 text-fd-primary">
                <Icon className="size-5" />
              </div>
              <h2 className="text-lg font-semibold">{item.title}</h2>
              <p className="mt-2 text-sm leading-relaxed text-fd-muted-foreground">{item.description}</p>
            </article>
          );
        })}
      </section>

      <section className="grid gap-6 lg:grid-cols-2">
        <article className="rounded-xl border bg-fd-card p-6">
          <h2 className="text-xl font-semibold">Quick Start</h2>
          <p className="mt-2 text-sm text-fd-muted-foreground">
            Run the docs locally and explore generated API references and guides.
          </p>
          <pre className="mt-4 overflow-x-auto rounded-md bg-fd-secondary p-4 text-xs sm:text-sm">
            <code>{startSteps.join('\n')}</code>
          </pre>
        </article>

        <article className="rounded-xl border bg-fd-card p-6">
          <h2 className="text-xl font-semibold">Navigate</h2>
          <div className="mt-4 space-y-3 text-sm">
            <Link
              href="/docs"
              className="group flex items-center justify-between rounded-md border p-3 transition-colors hover:bg-fd-accent"
            >
              <span className="inline-flex items-center gap-2 font-medium">
                <BookOpenText className="size-4" />
                User & API Documentation
              </span>
              <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link
              href="https://github.com/batleforc/proxyAuthK8s/discussions"
              className="group flex items-center justify-between rounded-md border p-3 transition-colors hover:bg-fd-accent"
            >
              <span className="inline-flex items-center gap-2 font-medium">
                <GitBranch className="size-4" />
                Discussions & Feedback
              </span>
              <ExternalLink className="size-4" />
            </Link>
          </div>
        </article>
      </section>
    </main>
  );
}
