#include <jni.h>
#include <espeak-ng/speak_lib.h>
#include <espeak-ng/espeak_ng.h>
#include <mutex>
#include <string>

extern "C" JNIEXPORT jstring JNICALL
Java_io_hookecho_HookEcho_PiperVoice_nativePhonemes(JNIEnv *env, jclass, jstring input,
                                                     jstring data_path) {
  static std::mutex lock;
  std::lock_guard<std::mutex> guard(lock);
  const char *text = env->GetStringUTFChars(input, nullptr);
  const char *path = env->GetStringUTFChars(data_path, nullptr);
  espeak_ng_InitializePath(path);
  espeak_ng_ERROR_CONTEXT error = nullptr;
  const auto status = espeak_ng_Initialize(&error);
  espeak_ng_ClearErrorContext(&error);
  if (status != ENS_OK || espeak_SetVoiceByName("en-us") != EE_OK) {
    env->ReleaseStringUTFChars(input, text);
    env->ReleaseStringUTFChars(data_path, path);
    env->ThrowNew(env->FindClass("java/lang/IllegalStateException"), "Unable to load eSpeak voice data");
    return nullptr;
  }
  const void *cursor = text;
  std::string result;
  while (cursor) {
    const char *part = espeak_TextToPhonemes(&cursor, espeakCHARS_UTF8, espeakPHONEMES_IPA);
    if (part) result += part;
  }
  env->ReleaseStringUTFChars(input, text);
  env->ReleaseStringUTFChars(data_path, path);
  return env->NewStringUTF(result.c_str());
}
