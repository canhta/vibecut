# Remote model selection

`AiPreferences` owns the bounded provider/feature choice map shared by Captions,
Shorts and the preference document. Each pair stores one selected ID and four
recent IDs. Its options put AI's shipped default first, then fetched, recent and
current IDs, without duplicates. All IDs use `ai::defaults::validate_model`.
Selection never fetches a list or needs a key.

`Catalogs` retains one explicitly requested refresh per provider. Selected-key
validation stays in engine's `Credentials`; a changed key cancels old work and
fences its result by generation. Polling does no IO. A worker catches panics,
returns the AI-owned bounded list or typed failure, and is joined on completion
or shutdown. The production transport timeout is 15 seconds; close may wait for
an in-flight request to return. Catalog lists and keys are transient, separate
from the model choice serialized by the preference document.
