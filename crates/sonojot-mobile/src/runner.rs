use crate::EngineError;

/// A speech recognizer the engine can drive. [`crate::SherpaRecognizer`] is the production runner;
/// other runtimes (or test doubles) implement the same three methods.
pub trait Recognizer: Send {
    /// Decodes one region of 16 kHz mono `f32` audio (at most [`Recognizer::max_segment_ms`] long)
    /// and returns the model's raw text.
    fn decode(&mut self, samples: &[f32]) -> Result<String, EngineError>;

    /// Identifies the loaded model in results and logs.
    fn model_id(&self) -> &str;

    /// The model's longest input window, when it has one.
    fn max_segment_ms(&self) -> Option<u32> {
        None
    }
}
