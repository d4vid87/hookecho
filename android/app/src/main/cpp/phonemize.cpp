#include <jni.h>
#include <espeak-ng/speak_lib.h>
#include <mutex>
#include <string>

extern "C" JNIEXPORT jstring JNICALL
Java_io_hookecho_HookEcho_PiperVoice_nativePhonemes(JNIEnv *env, jclass, jstring input,
                                                     jstring data_path) {
  static std::mutex lock;
  std::lock_guard<std::mutex> guard(lock);
  const char *text = env->GetStringUTFChars(input, nullptr);
  const char *path = env->GetStringUTFChars(data_path, nullptr);
  espeak_Initialize(AUDIO_OUTPUT_SYNCHRONOUS, 0, path, 0);
  espeak_SetVoiceByName("en-us");
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
