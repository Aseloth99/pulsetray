// A module hook for the tests: the project's imports have no file extension ("./prefs"), which plain Node needs.
export async function resolve(specifier, context, nextResolve) {
  if ((specifier.startsWith("./") || specifier.startsWith("../")) && !/\.[cm]?[jt]s$/.test(specifier)) {
    try {
      return await nextResolve(specifier + ".ts", context);
    } catch {
      /* not a .ts file: the default rules decide */
    }
  }
  return nextResolve(specifier, context);
}
