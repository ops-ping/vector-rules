#include "embedding_component.h"
#include "llama.h"
#define JSON_NOEXCEPTION 1
#include "nlohmann/json.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string>
#include <vector>

namespace {

using json = nlohmann::json;

constexpr const char * COMPONENT_ID = "embedding.wllama";
constexpr const char * COMPONENT_VERSION = "0.1.0";

struct Runtime {
    llama_model * model = nullptr;
    llama_context * context = nullptr;
    const llama_vocab * vocab = nullptr;
    std::string model_name;
    std::string model_revision;
    int32_t dimensions = 0;
    int32_t batch_size = 0;

    ~Runtime() {
        if (context != nullptr) {
            llama_free(context);
        }
        if (model != nullptr) {
            llama_model_free(model);
        }
    }
};

std::unique_ptr<Runtime> runtime;
bool backend_initialized = false;

bool fail(embedding_component_string_t * error, const std::string & message) {
    embedding_component_string_dup_n(error, message.data(), message.size());
    return false;
}

std::string as_string(const embedding_component_string_t * value) {
    return std::string(reinterpret_cast<const char *>(value->ptr), value->len);
}

std::string model_name_from_path(const std::string & path) {
    const size_t separator = path.find_last_of("/\\");
    const size_t start = separator == std::string::npos ? 0 : separator + 1;
    const size_t extension = path.find_last_of('.');
    const size_t end = extension == std::string::npos || extension < start
        ? path.size()
        : extension;
    return path.substr(start, end - start);
}

void set_info(exports_ai_vrules_embedding_embedding_info_t * info, const Runtime & state) {
    embedding_component_string_dup(&info->id, COMPONENT_ID);
    embedding_component_string_dup(&info->version, COMPONENT_VERSION);
    embedding_component_string_dup_n(&info->model, state.model_name.data(), state.model_name.size());
    embedding_component_string_dup_n(
        &info->revision,
        state.model_revision.data(),
        state.model_revision.size()
    );
    info->dimensions = static_cast<uint32_t>(state.dimensions);
}

bool tokenize(
    const Runtime & state,
    const std::string & text,
    std::vector<llama_token> & tokens,
    std::string & error
) {
    int32_t count = llama_tokenize(
        state.vocab,
        text.data(),
        static_cast<int32_t>(text.size()),
        nullptr,
        0,
        true,
        true
    );
    if (count >= 0) {
        error = "tokenizer did not report the required output size";
        return false;
    }

    tokens.resize(static_cast<size_t>(-count));
    count = llama_tokenize(
        state.vocab,
        text.data(),
        static_cast<int32_t>(text.size()),
        tokens.data(),
        static_cast<int32_t>(tokens.size()),
        true,
        true
    );
    if (count < 0) {
        error = "tokenizer result changed between sizing and tokenization";
        return false;
    }
    tokens.resize(static_cast<size_t>(count));
    return true;
}

bool embed_text(
    Runtime & state,
    const std::string & text,
    std::vector<float> & result,
    std::string & error
) {
    if (text.size() > static_cast<size_t>(INT32_MAX)) {
        error = "input text is too large";
        return false;
    }

    std::vector<llama_token> tokens;
    if (!tokenize(state, text, tokens, error)) {
        return false;
    }
    if (tokens.empty()) {
        error = "input produced no tokens";
        return false;
    }
    if (tokens.size() > static_cast<size_t>(state.batch_size)) {
        error = "input token count exceeds configured context size of " +
                std::to_string(state.batch_size);
        return false;
    }

    llama_batch batch = llama_batch_init(static_cast<int32_t>(tokens.size()), 0, 1);
    for (size_t index = 0; index < tokens.size(); ++index) {
        batch.token[index] = tokens[index];
        batch.pos[index] = static_cast<llama_pos>(index);
        batch.n_seq_id[index] = 1;
        batch.seq_id[index][0] = 0;
        batch.logits[index] = 1;
    }
    batch.n_tokens = static_cast<int32_t>(tokens.size());

    llama_memory_clear(llama_get_memory(state.context), true);
    const int32_t status = llama_decode(state.context, batch);
    llama_batch_free(batch);
    if (status != 0) {
        error = "llama.cpp embedding evaluation failed with status " + std::to_string(status);
        return false;
    }

    if (llama_pooling_type(state.context) == LLAMA_POOLING_TYPE_NONE) {
        error = "the GGUF model does not define sequence pooling";
        return false;
    }
    float * source = llama_get_embeddings_seq(state.context, 0);
    if (source == nullptr) {
        error = "llama.cpp returned no sequence embedding";
        return false;
    }

    result.assign(source, source + state.dimensions);
    double norm_squared = 0.0;
    for (float value : result) {
        norm_squared += static_cast<double>(value) * value;
    }
    const double norm = std::sqrt(norm_squared);
    if (!std::isfinite(norm) || norm == 0.0) {
        error = "llama.cpp returned an invalid embedding norm";
        return false;
    }
    for (float & value : result) {
        value = static_cast<float>(value / norm);
    }
    return true;
}

} // namespace

extern "C" bool exports_ai_vrules_embedding_initialize(
    embedding_component_string_t * config,
    exports_ai_vrules_embedding_embedding_info_t * info,
    embedding_component_string_t * error
) {
    if (runtime != nullptr) {
        return fail(error, "embedding component is already initialized");
    }

    const json parsed = json::parse(as_string(config), nullptr, false);
    if (parsed.is_discarded() || !parsed.is_object()) {
        return fail(error, "embedding config must be a JSON object");
    }
    const auto model_path_entry = parsed.find("model_path");
    if (model_path_entry == parsed.end() || !model_path_entry->is_string()) {
        return fail(error, "model_path must be a string");
    }
    const std::string model_path = model_path_entry->get<std::string>();
    if (model_path.empty()) {
        return fail(error, "model_path must not be empty");
    }

    std::string model_name = model_name_from_path(model_path);
    const auto model_entry = parsed.find("model");
    if (model_entry != parsed.end()) {
        if (!model_entry->is_string()) {
            return fail(error, "model must be a string");
        }
        model_name = model_entry->get<std::string>();
    }
    if (model_name.empty()) {
        return fail(error, "model must not be empty");
    }
    const auto revision_entry = parsed.find("model_sha256");
    if (revision_entry == parsed.end() || !revision_entry->is_string()) {
        return fail(error, "model_sha256 must be a string");
    }
    const std::string model_revision = revision_entry->get<std::string>();
    if (model_revision.size() != 64 || !std::all_of(
            model_revision.begin(),
            model_revision.end(),
            [](unsigned char value) { return std::isxdigit(value) != 0; }
        )) {
        return fail(error, "model_sha256 must contain 64 hexadecimal characters");
    }

    int32_t context_size = 2048;
    const auto context_entry = parsed.find("context_size");
    if (context_entry != parsed.end()) {
        if (!context_entry->is_number_integer()) {
            return fail(error, "context_size must be an integer");
        }
        const int64_t requested = context_entry->get<int64_t>();
        if (requested <= 0 || requested > 32768) {
            return fail(error, "context_size must be between 1 and 32768");
        }
        context_size = static_cast<int32_t>(requested);
    }

    enum llama_pooling_type pooling_type = LLAMA_POOLING_TYPE_UNSPECIFIED;
    const auto pooling_entry = parsed.find("pooling");
    if (pooling_entry != parsed.end()) {
        if (!pooling_entry->is_string()) {
            return fail(error, "pooling must be a string");
        }
        const std::string value = pooling_entry->get<std::string>();
        if (value == "mean") {
            pooling_type = LLAMA_POOLING_TYPE_MEAN;
        } else if (value == "cls") {
            pooling_type = LLAMA_POOLING_TYPE_CLS;
        } else if (value == "last") {
            pooling_type = LLAMA_POOLING_TYPE_LAST;
        } else {
            return fail(error, "pooling must be one of: mean, cls, last");
        }
    }

    if (!backend_initialized) {
        llama_backend_init();
        backend_initialized = true;
    }

    llama_model_params model_params = llama_model_default_params();
    model_params.n_gpu_layers = 0;
    model_params.use_mmap = false;
    model_params.use_direct_io = false;
    model_params.use_mlock = false;

    std::unique_ptr<Runtime> state(new (std::nothrow) Runtime());
    if (state == nullptr) {
        return fail(error, "allocate embedding runtime");
    }
    state->model = llama_model_load_from_file(model_path.c_str(), model_params);
    if (state->model == nullptr) {
        return fail(error, "failed to load GGUF model at " + model_path);
    }
    state->model_revision = model_revision;

    llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = static_cast<uint32_t>(context_size);
    context_params.n_batch = static_cast<uint32_t>(context_size);
    context_params.n_ubatch = static_cast<uint32_t>(context_size);
    context_params.n_seq_max = 1;
    context_params.n_threads = 1;
    context_params.n_threads_batch = 1;
    context_params.embeddings = true;
    context_params.pooling_type = pooling_type;

    state->context = llama_init_from_model(state->model, context_params);
    if (state->context == nullptr) {
        return fail(error, "failed to create llama.cpp embedding context");
    }
    if (llama_pooling_type(state->context) == LLAMA_POOLING_TYPE_NONE) {
        return fail(error, "the GGUF model does not define sequence pooling");
    }

    state->vocab = llama_model_get_vocab(state->model);
    state->dimensions = llama_model_n_embd_out(state->model);
    state->batch_size = context_size;
    state->model_name = model_name;
    if (state->vocab == nullptr || state->dimensions <= 0) {
        return fail(error, "the GGUF model has invalid embedding metadata");
    }

    set_info(info, *state);
    runtime = std::move(state);
    return true;
}

extern "C" bool exports_ai_vrules_embedding_embed(
    embedding_component_string_t * text,
    embedding_component_list_f32_t * output,
    embedding_component_string_t * error
) {
    if (runtime == nullptr) {
        return fail(error, "embedding component is not initialized");
    }
    std::vector<float> result;
    std::string message;
    if (!embed_text(*runtime, as_string(text), result, message)) {
        return fail(error, message);
    }
    output->len = result.size();
    output->ptr = static_cast<float *>(std::malloc(result.size() * sizeof(float)));
    if (output->ptr == nullptr) {
        return fail(error, "allocate embedding result");
    }
    std::memcpy(output->ptr, result.data(), result.size() * sizeof(float));
    return true;
}
